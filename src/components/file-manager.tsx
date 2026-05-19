import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  useFileManagerStore,
  type ViewMode,
} from "@/stores/file-manager-store";
import { formatBytes } from "@/lib/format-bytes";
import { useUIStore } from "@/stores/ui-store";
import { useAiStore, type AiExecutionResult } from "@/stores/ai-store";
import { useAutomationStore } from "@/stores/automation-store";
import { useSpacesStore } from "@/stores/spaces-store";
import { useTerminalStore } from "@/stores/terminal-store";
import { isTauriAvailable, tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { loadSmartDestinations, fileExtension, type SmartDestination } from "@/hooks/use-smart-destinations";
import { assessBeforeExecute } from "@/lib/safety";
import { copyToClipboardWithToast } from "@/lib/clipboard";
import { reportOperationFailure } from "@/lib/op-error-toast";
import {
  computeListBytes,
  computeSelectionBytes,
  formatPaneStatsLabel,
} from "@/lib/selection-stats";
import { archiveExtractDest } from "@/lib/archive-paths";
import {
  useDriveSpace,
  useAllDrives,
  invalidateDriveCache,
  refreshDriveCacheIfStale,
} from "@/hooks/use-drive-space";
import {
  formatDriveSpace,
  formatDriveSpaceDetail,
} from "@/lib/drive-space";
import {
  dispatchLedgerPath,
  deriveLabel,
  labelForKind,
  formatRelativeTime,
  pickJumpRingTarget,
  type JumpRingDirection,
} from "@/lib/ledger-dispatch";
import { DualPaneLayout } from "./dual-pane-layout";
import { TabBar } from "./tab-bar";
import { BreadcrumbBar } from "./breadcrumb-bar";
import { SidebarNav } from "./sidebar-nav";
import { FileList, type FileEntryData } from "./file-list";
import { FilterBar, filterFiles } from "./filter-bar";
import { ContextMenu, getFileContextMenuItems, type ContextMenuItem } from "./context-menu";
import { CommandPalette, getDefaultCommands, type CommandItem } from "./command-palette";
import { KeyboardCheatSheet } from "./keyboard-cheat-sheet";
import { BatchRename } from "./batch-rename";
import SettingsPanel from "./settings-panel";
import { ConnectionPanel } from "./connection-panel";
import { SyncPanel } from "./sync-panel";
import { PreviewPane } from "./preview-pane";
import { TransferHistoryPanel } from "./transfer-history-panel";
import { ArchiveBrowser, CreateArchiveDialog } from "./archive-browser";
import { TextEditorModal } from "./text-editor-modal";
import { CompareFilesModal, type CompareData } from "./compare-files-modal";
import { IntegrityTools } from "./integrity-tools";
import {
  dispatchRefresh,
  shouldRefreshPane,
  parentDirectoriesOf,
  REFRESH_EVENT,
  type RefreshEventDetail,
} from "@/lib/refresh-affected";
import { useLedgerTailPoll } from "@/hooks/use-ledger-tail-poll";
import { AiPanel } from "./ai-panel";
import { AutomationPanel } from "./automation-panel";
import { SmartSpaceWizard } from "./smart-space-wizard";
import { TerminalPanel } from "./terminal-panel";
import { SinceLastSeenToast, type LedgerSinceSummary } from "./since-last-seen-toast";
import { ActivityTimelinePanel } from "./activity-timeline-panel";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";
import { LineagePanel } from "./lineage-panel";
import { useLineageStore, type FileLineage } from "@/stores/lineage-store";
import {
  inferPathRecall,
  describePathRecall,
  type PathRecallInfo,
} from "@/lib/path-recall";
import { PathJumpDialog } from "./path-jump-dialog";
import { usePathJumpStore } from "@/stores/path-jump-store";
import { SafetyInterlockDialog } from "./safety-interlock-dialog";
import { EnginePulseIndicator } from "./engine-pulse-indicator";
import { useDirectoryActivity, useRecentlyTouchedSet } from "@/hooks/use-directory-activity";
import { useFileSelection, useFileDragDrop } from "@/hooks/use-file-selection";
import { cn } from "@ufop/ui-components";
import { Button, Badge } from "@ufop/ui-components";
import { ThemeSwitcher } from "./theme-switcher";
import {
  FolderOpen,
  PanelLeft,
  List,
  AlignJustify,
  Grid3X3,
  Layout,
  Undo2,
  Bot,
  Terminal,
  Eye,
  EyeOff,
  Link2,
  Unlink2,
  Save,
  FolderDown,
  Zap,
  Layers,
} from "lucide-react";

// Toolbar customization definition (#14)
const ALL_TOOLBAR_ITEMS = [
  { id: "sidebar", label: "Sidebar Toggle" },
  { id: "view-modes", label: "View Mode Selector" },
  { id: "hidden-files", label: "Hidden Files Toggle" },
  { id: "ai", label: "AI Assistant" },
  { id: "terminal", label: "Terminal" },
  { id: "theme", label: "Theme Switcher" },
  { id: "undo", label: "Undo Button" },
  { id: "automations", label: "Quickflows" },
  { id: "smart-spaces", label: "Smart Spaces" },
  { id: "sync-browse", label: "Sync Browse" },
];

// ──────────────────────────────────────────────
// FileManager - main orchestrating component
// ──────────────────────────────────────────────

/**
 * Main file manager component that composes all sub-components:
 * - Dual-pane layout (T-006)
 * - Directory listing (T-007)
 * - Tabbed browsing (T-008)
 * - Navigation sidebar (T-009)
 * - File operations (T-010, wired to Tauri IPC)
 * - Multi-select and DnD (T-011)
 * - Search and filter (T-012)
 * - Context menus, keyboard nav, command palette (T-013)
 */

/** Iter 37: module-level extension predicates shared between
 *  `FilePane.handleOpen` (the double-click path from iter 31-32)
 *  and the `PathJumpDialog` file-aware routing (iter 37). Defined
 *  here rather than in a separate file to keep iter-37 scope
 *  contained to file-manager.tsx + path-jump-dialog.tsx. Lists
 *  mirror the backend capability matrix — archives match the
 *  `archive_browse` match arm in archive_commands.rs, text
 *  matches the whitelist documented in text-editor-modal.tsx. */
const SMART_OPEN_ARCHIVE_EXT = [
  ".zip",
  ".tar",
  ".tar.gz",
  ".tgz",
  ".7z",
] as const;

const SMART_OPEN_TEXT_EXT = [
  ".txt",
  ".md",
  ".markdown",
  ".json",
  ".yaml",
  ".yml",
  ".toml",
  ".xml",
  ".csv",
  ".log",
  ".conf",
  ".ini",
  ".env",
] as const;

/** Iter 38: media files the iter-29 `PreviewPane` can render inline.
 *  Covers the four `PreviewType` variants produced by the backend's
 *  `preview_file` IPC (Image, Pdf, Audio, Video). Deliberately omits
 *  Markdown (which iter 32 routes to the editor — editing beats
 *  rendering for the double-click gesture) and archive formats
 *  (iter 31 routes them to the in-app archive browser). Order of
 *  precedence in `handleOpen`: archive > text > preview > OS
 *  default — a `.md` will always hit the editor, a `.tar.gz` will
 *  always hit the browser, and a `.jpg` only lands in the preview
 *  pane when that pane is already visible (see the contextual
 *  guard below). */
const SMART_OPEN_PREVIEW_EXT = [
  // Images
  ".jpg",
  ".jpeg",
  ".png",
  ".gif",
  ".webp",
  ".svg",
  ".bmp",
  ".ico",
  // PDF
  ".pdf",
  // Audio
  ".mp3",
  ".wav",
  ".ogg",
  ".flac",
  ".m4a",
  ".aac",
  // Video
  ".mp4",
  ".mkv",
  ".mov",
  ".webm",
  ".avi",
] as const;

function isArchivePath(path: string): boolean {
  const lower = path.toLowerCase();
  return SMART_OPEN_ARCHIVE_EXT.some((ext) => lower.endsWith(ext));
}

function isPlainTextPath(path: string): boolean {
  const lower = path.toLowerCase();
  return SMART_OPEN_TEXT_EXT.some((ext) => lower.endsWith(ext));
}

function isPreviewablePath(path: string): boolean {
  const lower = path.toLowerCase();
  return SMART_OPEN_PREVIEW_EXT.some((ext) => lower.endsWith(ext));
}

/** Iter 43: Jump Ring state for the ⌘⇧L shortcut. Deliberately
 *  module-scoped rather than a React `useRef` — FileManager is the
 *  root orchestrator (exactly one instance at runtime), and hoisting
 *  this out of the component keeps React Compiler from pessimistically
 *  flagging ref access in transitively-called callbacks. Mutated
 *  imperatively from inside `handleJumpLastTouched` but never read
 *  during render, so React's reconciler never needs to be aware of it.
 *
 *  - `index`         : current position in the `distinctPaths` ring
 *  - `lastPressTime` : last invocation timestamp; used to detect
 *                      rapid re-presses (<2500ms)
 *  - `distinctPaths` : snapshot of the distinct-recent list on the
 *                      last press, so a fresh ledger event between
 *                      presses can't confuse cycling direction */
const jumpRing: {
  index: number;
  lastPressTime: number;
  distinctPaths: string[];
} = { index: 0, lastPressTime: 0, distinctPaths: [] };

export function FileManager() {
  const appMode = useUIStore((s) => s.appMode);
  const sidebarOpen = useUIStore((s) => s.sidebarOpen);
  const sidebarWidth = useUIStore((s) => s.sidebarWidth);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const commandPaletteOpen = useFileManagerStore((s) => s.commandPaletteOpen);
  const setCommandPaletteOpen = useFileManagerStore((s) => s.setCommandPaletteOpen);
  const undoStack = useFileManagerStore((s) => s.undoStack);
  const popUndo = useFileManagerStore((s) => s.popUndo);
  // Live-subscribed so the command palette's per-bookmark "Jump to …"
  // entries refresh whenever the user adds, removes, or reorders a
  // favorite. Driving the palette directly off the favorites ring keeps
  // the bookmark surface DRY: zero parallel state, zero sync code.
  const favorites = useFileManagerStore((s) => s.favorites);

  // AI panel state (T-043..T-045)
  const aiPanelOpen = useAiStore((s) => s.panelOpen);
  const toggleAiPanel = useAiStore((s) => s.togglePanel);

  // Automation panel (Quickflows)
  const automationPanelOpen = useAutomationStore((s) => s.panelOpen);
  const toggleAutomationPanel = useAutomationStore((s) => s.togglePanel);
  // Quickflow Launchpad — surface every saved manual-trigger rule in
  // the existing Command Palette so the user can fuzzy-search the rule
  // name and press Enter to fire it. The rules array is loaded eagerly
  // on mount (via the useEffect below) so the palette is always ready,
  // even when the user has never opened the automation panel in this
  // session.
  const automationRules = useAutomationStore((s) => s.rules);
  const loadAutomationRules = useAutomationStore((s) => s.loadRules);
  const runAutomationRule = useAutomationStore((s) => s.runRule);

  // Activity Timeline panel (unified ledger viewer)
  const toggleActivityTimeline = useActivityTimelineStore((s) => s.togglePanel);
  const activityTimelineOpen = useActivityTimelineStore((s) => s.panelOpen);
  // Lineage panel opens lazily once a file is selected for inspection.
  const lineagePanelOpen = useLineageStore((s) => s.pendingPath !== null);

  // Smart Spaces
  const spacesWizardOpen = useSpacesStore((s) => s.wizardOpen);
  const closeSpacesWizard = useSpacesStore((s) => s.closeWizard);
  const openSpacesWizard = useSpacesStore((s) => s.openWizard);

  // Terminal state (T-046)
  const terminalPanelOpen = useTerminalStore((s) => s.panelOpen);
  const toggleTerminalPanel = useTerminalStore((s) => s.togglePanel);
  const createLocalSession = useTerminalStore((s) => s.createLocalSession);

  // Sync browsing (#38)
  const syncBrowsing = useFileManagerStore((s) => s.syncBrowsing);
  const toggleSyncBrowsing = useFileManagerStore((s) => s.toggleSyncBrowsing);
  const singlePaneMode = useFileManagerStore((s) => s.singlePaneMode);

  // Iter 17: live drive list for the sidebar's Devices section. The
  // hook reuses iter-16's 30s-TTL cache, so this single subscription
  // amortises across the status-bar free-space lookup and any other
  // mount-table consumer. The mapping below normalises the backend
  // field names (`mount_point`, `total_bytes`, `free_bytes`) to the
  // older `path`/`totalSpace`/`freeSpace` shape `SidebarNav` was
  // built around — kept inline because it's the *only* place the
  // two shapes meet, and a one-purpose helper would be more cognitive
  // overhead than the four-key object literal it replaces.
  const allDrives = useAllDrives();
  const sidebarDrives = useMemo(
    () =>
      allDrives.map((d) => ({
        name: d.name,
        path: d.mount_point,
        totalSpace: d.total_bytes,
        freeSpace: d.free_bytes,
        removable: d.removable,
        // Iter 19: pre-compute the rich tooltip at the boundary where
        // backend snake_case meets frontend camelCase. Reuses iter 16's
        // `formatDriveSpaceDetail` so the sidebar hover and the
        // status-bar free-space hover speak the same language. Falls
        // back to "name (mount_point)" when the drive has no space
        // accounting (network mounts, optical drives) so power users
        // can still see the full mount path on hover.
        tooltip:
          formatDriveSpaceDetail(d) ?? `${d.name} (${d.mount_point})`,
      })),
    [allDrives],
  );

  // Toolbar customization (#14)
  const toolbarItems = useUIStore((s) => s.toolbarItems);
  const setToolbarItems = useUIStore((s) => s.setToolbarItems);
  const resetToolbar = useUIStore((s) => s.resetToolbar);
  const toolbarCustomizerOpen = useUIStore((s) => s.toolbarCustomizerOpen);
  const toggleToolbarCustomizer = useUIStore((s) => s.toggleToolbarCustomizer);
  // Iter 29: subscribe to the ui-store's preview slot so React
  // re-renders the modal when `openPreviewFor` / `closePreview`
  // fire from the keyboard handler or palette action.
  const previewPanelOpen = useUIStore((s) => s.previewPanelOpen);
  const previewFilePath = useUIStore((s) => s.previewFilePath);

  // Workspace management
  const workspaces = useFileManagerStore((s) => s.workspaces);
  const saveWorkspace = useFileManagerStore((s) => s.saveWorkspace);
  const loadWorkspace = useFileManagerStore((s) => s.loadWorkspace);
  const deleteWorkspace = useFileManagerStore((s) => s.deleteWorkspace);
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);
  // Iter 25: Batch Rename dialog state. `null` when closed, an
  // array of selected file paths when open. Declared at the top
  // of the orchestrator (rather than next to the selection
  // helpers) so the Cmd+Shift+R keyboard handler in the global
  // `useEffect` below can reference `setBatchRenameFiles`
  // without a temporal-dead-zone lint error.
  const [batchRenameFiles, setBatchRenameFiles] = useState<string[] | null>(
    null,
  );
  // Iter 26: Settings panel modal state. The SettingsPanel
  // component (1354 LOC of export/import + sync + notification
  // prefs + privacy + logs) was built as an orphan \u2014 no file
  // imported it. Iter 26 wires it via the universal Cmd+,
  // keyboard shortcut and a palette entry. Declared at the top
  // of the component for the same TDZ reason as
  // `batchRenameFiles`: the keyboard handler in the useEffect
  // below needs the setter in scope.
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Iter 27: Connection manager modal state. ConnectionPanel is
  // 1941 LOC of production-ready UI for managing connection
  // profiles across the 17 connector protocols (SFTP, S3,
  // Google Drive, WebDAV, SMB, FTP, Azure Blob, GCS, Dropbox,
  // Box, OneDrive, iCloud Drive, Mega, pCloud, Nextcloud,
  // OpenStack Swift, MinIO). All 17 Tauri commands it uses are
  // pre-registered in the Rust backend; the component itself
  // just had zero importers. Iter 27 surfaces it via
  // Cmd+Shift+N and a palette entry. Same TDZ-safe placement
  // as the iter 25/26 orphan dialogs.
  const [connectionsOpen, setConnectionsOpen] = useState(false);
  // Iter 28: Sync panel modal state. SyncPanel is 1454 LOC of
  // sync-pair management UI \u2014 health indicators, create-pair
  // wizard (simple + advanced), dry-run preview with category
  // counts (additions/modifications/deletions/conflicts),
  // execution with progress, conflict resolution, reports,
  // and rollback. All 14 sync IPC commands
  // (list/create/delete/dry_run/run/rollback/get_conflicts/
  // resolve_conflict/get_health/get_reports/export_csv/
  // export_json/validate_cron) were pre-registered in the
  // Rust backend; the component just had zero importers.
  // Iter 28 surfaces it via Cmd+Shift+H ("sync Health") and a
  // palette entry. Same TDZ-safe placement as iters 25\u201327.
  const [syncOpen, setSyncOpen] = useState(false);
  // Iter 29: PreviewPane orphan elevation. The 844-LOC
  // component was never imported anywhere AND the ui-store
  // already had a pre-existing half-wired slot
  // (`previewPanelOpen` + `togglePreviewPanel` +
  // `previewPanelWidth`) that was declared, persisted, and
  // covered by tests but never consumed by any component.
  // Iter 29 extends the store with the missing `previewFilePath`
  // + `openPreviewFor` / `closePreview` actions and routes the
  // new Cmd+Shift+P gesture through them. This single source
  // of truth is persisted via Zustand's `persist` middleware,
  // so opening preview on foo.pdf \u2192 quit \u2192 relaunch re-opens
  // preview on foo.pdf automatically.
  // Iter 20: backend ledger tail poll. Watches the unified operation
  // ledger every few seconds and fires `dispatchRefresh` for paths
  // touched by BACKEND-initiated mutations (transfer engine, sync
  // engine, automation rules, watcher, safety interlock). Pairs
  // with iter 18/19 which covered frontend-initiated mutations \u2014
  // together they guarantee that any mutation landing in the ledger
  // propagates to every pane whose viewed directory intersects.
  // Hook is mount-once per app: it runs for as long as the
  // file manager is open and pauses when the tab is hidden.
  useLedgerTailPoll();
  // Iter 17: keyboard cheat sheet overlay. Purely UI state — the
  // overlay reads the existing `commands` array and renders every
  // palette entry with a `shortcut` field, so it auto-updates as
  // future iterations register more keyboard gestures. Local
  // `useState` keeps the concern out of the Zustand store since
  // there is nothing to persist across app restarts.
  const [cheatSheetOpen, setCheatSheetOpen] = useState(false);
  // Iter 30: Transfer History panel modal state. The
  // `TransferHistoryPanel` component (284 LOC) was built for
  // T-018 with three backend IPCs already registered —
  // `search_transfer_history`, `export_transfer_history`,
  // `cleanup_transfer_history` — but no file imported it. Iter 30
  // surfaces it via Cmd+Shift+J ("J for Journal" — transfer history
  // is effectively a transfer journal; J was the only still-free
  // letter that carried semantic meaning after the iter 12-29
  // accumulation). Same TDZ-safe placement as iters 25-29 so the
  // keyboard handler below can reference the setter in scope.
  const [transferHistoryOpen, setTransferHistoryOpen] = useState(false);

  // Iter 31: Contextual elevation of the `ArchiveBrowser` orphan
  // (680 LOC). Unlike iters 25-30 which added global Cmd+Shift
  // modals, iter 31 surfaces the component only when the user
  // double-clicks an archive file — respecting the component's
  // original design intent ("Double-click archive opens as virtual
  // folder") and avoiding further modal-keyboard-shortcut overload.
  // Four backend IPCs (archive_browse, archive_info, archive_extract,
  // archive_create) are already registered in lib.rs. A null value
  // means the panel is closed; a string is the archive path being
  // browsed. `handleOpen` intercepts archive extensions and sets
  // this state instead of handing off to `open_file_with_default`.
  const [archiveBrowserPath, setArchiveBrowserPath] = useState<string | null>(null);

  // Iter 32: Contextual elevation of the `TextEditor` orphan (234
  // LOC). Extends iter 31's "smart double-click routing" pattern to
  // plain-text/config files (.txt, .md, .json, .yaml, .toml, .xml,
  // .csv, .log, .conf, .ini, .env) by hosting the editor in a modal
  // driven by two new IPCs added this iter in `fs_commands.rs`:
  // `read_text_file_full` and `write_text_file_full`. Stores both
  // the path AND the file size so the TextEditor's own 1 MB
  // too-large guard can render without a round-trip. Code files
  // (.js, .ts, .py, etc.) are deliberately NOT intercepted — users
  // typically have strong preferences about which IDE handles them
  // and we don't want to bypass those preferences from a double-
  // click.
  const [textEditorFile, setTextEditorFile] = useState<{ path: string; size: number } | null>(null);

  // Iter 36: Integrity Tools panel modal state. The
  // `IntegrityTools` component (1071 LOC, T-063) was the last
  // clean orphan with full backend wiring — five pre-registered
  // IPCs (integrity_checksum, integrity_verify, compute_checksum,
  // integrity_find_duplicates, integrity_get_file_info) covering
  // four tabs: Checksums, Duplicates, Tags & Labels, Smart
  // Folders. Surfaced via Cmd+Shift+I ("I for Integrity") and a
  // palette entry; file-selection context is passed as the
  // active pane's current directory so the Duplicates and Smart
  // Folder tabs work out-of-the-box. Future iters may add a
  // context-menu path that threads selectedFiles from the active
  // pane's selection for the Checksums tab. Storing the directory
  // snapshot in state (null = closed) lets the keyboard handler
  // remain TDZ-safe alongside the iter-25..32 orphan states.
  // Iter 39: extended from iter 36's string|null shape to carry
  // BOTH the current-directory baseline AND the pre-selected
  // files, so the context-menu entry "Verify Integrity…" can
  // pass the right-clicked selection directly into the
  // Checksums tab without forcing the user to manually
  // re-select inside the modal. `files: []` for the Cmd+Shift+I
  // and palette paths preserves iter-36 behaviour.
  const [integrityToolsOpen, setIntegrityToolsOpen] = useState<{
    dir: string;
    files: string[];
  } | null>(null);

  // Iter 41: Compress to archive. Mirrors the integrity-tools modal
  // pattern (singleton state at FileManager scope; `null` means
  // closed; FilePane signals up via the new `onCompress` prop).
  // Hosts the `CreateArchiveDialog` orphan that was already built in
  // archive-browser.tsx — backend `archive_create` IPC has been
  // wired since T-062, only the entry point was missing.
  const [createArchiveSources, setCreateArchiveSources] = useState<
    string[] | null
  >(null);

  const handleSaveWorkspace = useCallback(() => {
    const name = window.prompt("Save workspace as:", `Workspace ${workspaces.length + 1}`);
    if (name?.trim()) {
      saveWorkspace(name.trim());
    }
  }, [workspaces.length, saveWorkspace]);

  const handleLoadWorkspace = useCallback((id: string) => {
    loadWorkspace(id);
    setWorkspaceMenuOpen(false);
  }, [loadWorkspace]);

  const handleDeleteWorkspace = useCallback((id: string) => {
    deleteWorkspace(id);
  }, [deleteWorkspace]);

  // Intent Bar execution result handler
  const handleIntentResult = useCallback((result: AiExecutionResult) => {
    if (!result.success) return;

    const action = result.action_taken;

    if (action === "filter" || action === "navigate") {
      // Frontend-only: navigate or filter
      if (result.entity_id) {
        try {
          const info = JSON.parse(result.entity_id);
          if (info.path) {
            // Navigate to the path
            const store = useFileManagerStore.getState();
            const pane = store.panes[store.activePaneIndex];
            const tab = pane?.tabs.find((t) => t.id === pane.activeTabId);
            if (tab) {
              store.navigateTab(store.activePaneIndex, tab.id, info.path, info.path.split("/").pop() || info.path);
            }
          }
        } catch {
          // not parseable, ignore
        }
      }
    } else if (action === "vault") {
      // Open vault panel — no separate panel toggle exists yet, but we could open AI panel
      // For now, just log
      console.log("Intent: open vault for", result.entity_id);
    } else if (action.startsWith("sync:") || action.startsWith("transfer:")) {
      // Show a toast-like message via structured error (info level)
      const uiStore = useUIStore.getState();
      uiStore.addStructuredError({
        what: action.includes("sync") ? "Sync pair configured" : "Transfer configured",
        why: "Created from your natural language request",
        appDid: result.action_taken,
        userAction: "Open the Sync panel to review and activate",
      });
    } else if (action.startsWith("Created automation")) {
      const uiStore = useUIStore.getState();
      uiStore.addStructuredError({
        what: "Automation rule created",
        why: "Created from your natural language request",
        appDid: action,
        userAction: "Open Quickflows panel to review and enable",
      });
    }
  }, []);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    position: { x: number; y: number };
    items: ContextMenuItem[];
  } | null>(null);

  // Universal Time-Travel Undo — talks to the ledger-backed `undo_last`
  // command so Cmd+Z works across app restarts and across every engine that
  // records to the unified ledger, not just the in-session memory stack.
  //
  // Outside Tauri we fall back to the legacy in-memory stack so the Vite
  // browser preview still echoes something to the console when Cmd+Z is
  // pressed. Inside Tauri the in-memory `popUndo()` is still invoked to keep
  // the visible undoStack badge count in sync with what the backend just
  // reversed — it's a display hint, not the source of truth.
  const handleUndo = useCallback(async () => {
    if (!isTauriAvailable()) {
      const entry = popUndo();
      if (entry) console.log("Undo (demo mode):", entry);
      return;
    }
    try {
      const outcome = await tauriInvoke<{
        success: boolean;
        correlation_id: string;
        kind: string;
        summary: string;
        item_count: number;
      }>("undo_last", undefined, {
        success: false,
        correlation_id: "",
        kind: "",
        summary: "Nothing to undo",
        item_count: 0,
      });
      // Keep the optimistic in-memory stack in sync with real history.
      if (outcome.success) popUndo();
      // Surface result via the existing structured-error toast channel so
      // we don't introduce a new notification system.
      useUIStore.getState().addStructuredError({
        what: outcome.success
          ? `Undid ${outcome.kind}`
          : "Nothing to undo",
        why: outcome.success
          ? outcome.summary
          : "The operation ledger has no reversible entries from this or prior sessions.",
        appDid: outcome.success
          ? `Reversed ${outcome.item_count} item(s) via operation ledger`
          : "No-op",
        userAction: outcome.success
          ? "Press Cmd+Z again to undo the next most recent operation"
          : "Perform a file operation first, then Cmd+Z will reverse it",
      });
    } catch (err) {
      console.error("Undo failed:", err);
      useUIStore.getState().addStructuredError({
        what: "Undo failed",
        why: String(err),
        appDid: "The backend rejected the undo request",
        userAction: "Check the Activity Timeline for the most recent operation and try the per-entry Undo button",
      });
    }
  }, [popUndo]);

  // Universal Time-Travel Redo — symmetric mirror of `handleUndo`. Cmd+Shift+Z
  // re-applies the most recent undone operation via the ledger-backed
  // `redo_last` command. Pure additive: users who never press Cmd+Shift+Z
  // see no change. Outside Tauri this is a no-op (the demo undo stack has
  // no redo concept), matching the behaviour of every other ledger-bound
  // shortcut in this file.
  const handleRedo = useCallback(async () => {
    if (!isTauriAvailable()) return;
    try {
      const outcome = await tauriInvoke<{
        success: boolean;
        correlation_id: string;
        kind: string;
        summary: string;
        item_count: number;
      }>("redo_last", undefined, {
        success: false,
        correlation_id: "",
        kind: "",
        summary: "Nothing to redo",
        item_count: 0,
      });
      useUIStore.getState().addStructuredError({
        what: outcome.success
          ? `Redid ${outcome.kind}`
          : "Nothing to redo",
        why: outcome.success
          ? outcome.summary
          : "There is no undone operation to re-apply.",
        appDid: outcome.success
          ? `Re-applied ${outcome.item_count} item(s) via operation ledger`
          : "No-op",
        userAction: outcome.success
          ? "Press Cmd+Shift+Z again to redo the next most recent undo"
          : "Undo an operation first (Cmd+Z), then Cmd+Shift+Z will replay it",
      });
    } catch (err) {
      console.error("Redo failed:", err);
      useUIStore.getState().addStructuredError({
        what: "Redo failed",
        why: String(err),
        appDid: "The backend rejected the redo request",
        userAction: "Check the Activity Timeline for the most recent undo and verify the source files still exist",
      });
    }
  }, []);

  // Iter 46: single-path dispatch handler that routes any ledger
  // path through the shared `dispatchLedgerPath` helper. Used by:
  //   - Activity Timeline row clicks (iter 46)
  //   - Future surfaces that hand us a ledger path and want the
  //     same smart-routing as Instant Jump / Jump Ring
  // Centralizing the callback setup here means ALL ledger-sourced
  // navigation routes through ONE code path that knows about
  // archives, the in-app text editor, the preview pane, and the
  // active-tab navigation. Zero new backend, zero new IPC.
  const handleDispatchLedgerPath = useCallback(async (path: string) => {
    await dispatchLedgerPath(path, {
      isArchivePath,
      isPlainTextPath,
      isPreviewablePath,
      onOpenArchive: (p) => setArchiveBrowserPath(p),
      onOpenFile: (p, size) => setTextEditorFile({ path: p, size }),
      onOpenPreview: (p) => useUIStore.getState().openPreviewFor(p),
      onNavigateDir: (p) => {
        const store = useFileManagerStore.getState();
        const pIdx = store.activePaneIndex;
        const tab = store.getActiveTab(pIdx);
        if (!tab) return;
        store.navigateTab(pIdx, tab.id, p, deriveLabel(p));
      },
    });
  }, []);

  // Iter 42/43: "Jump to Last Touched" shared handler. Iter 42
  // fetched the single top row of `ledger_recent_paths` and
  // dispatched it. That worked when the user had just done work
  // in a different folder, but produced a silent no-op when the
  // top hit resolved back to the folder the user was currently
  // sitting in (e.g. they just edited a file in-place). Iter 43
  // upgrades the handler to a "Jump Ring":
  //
  //   - Fetch top 25 hits (cheap SQL scan, never exceeds the
  //     modal's result cap) and dedup by path.
  //   - First press: skip any hit matching the active tab's
  //     current path so the gesture always takes the user
  //     somewhere *different*.
  //   - Rapid re-press (<2500ms): cycle to the next distinct
  //     path in the ring, alt-tab style. After the 2500ms
  //     window the ring resets so the next press means "take
  //     me to the newest recent" again.
  //   - Fallback: if filtering removes every hit (unrealistic —
  //     would require the top 25 all being the current path),
  //     the handler dispatches the raw top hit exactly the way
  //     iter 42 did, preserving that surface.
  //
  // The ring state lives in the module-level `jumpRing` object
  // (see comment at the top of the file) so React's render cycle
  // never sees it and the React Compiler's "cannot access refs
  // during render" rule stays silent. Palette clicks fall through
  // the first-press branch because they can't meaningfully
  // re-press within the cycle window.
  const handleJumpLastTouched = useCallback(
    async (direction: JumpRingDirection = "forward") => {
    // Iter 44: fetch the enriched hit shape so the post-dispatch
    // "operation preview" toast can explain WHY the destination
    // ranks where it does (kind + "Nm ago"). `last_kind` was
    // added in iter 40, `last_seen` has existed since iter 37.
    const hits = await tauriInvokeSafe<
      {
        path: string;
        last_seen: string;
        hit_count: number;
        last_kind?: string;
      }[]
    >("ledger_recent_paths", { limit: 25, query: null }, []);
    if (hits.length === 0) {
      useUIStore.getState().addStructuredError({
        what: "No recent paths yet",
        why: "The operation ledger hasn't recorded any file or folder activity yet",
        appDid: "Skipped the jump because there was nothing to jump to",
        userAction:
          "Perform any file operation (copy, move, edit, create) and try again",
      });
      return;
    }
    // Resolve the active tab's current path so the pure ring
    // picker can filter it out. Reads `getState()` inline to
    // avoid a stale-closure hazard inside the useCallback.
    const fmStore = useFileManagerStore.getState();
    const paneIndex = fmStore.activePaneIndex;
    const activeTab = fmStore.getActiveTab(paneIndex);
    const currentPath = activeTab?.path ?? "";

    // Iter 45: ring decision lives in a pure function now.
    // `pickJumpRingTarget` handles empty / all-current / cycling
    // forward / cycling reverse / reset — the file-manager just
    // hands it the inputs and commits the returned state.
    const outcome = pickJumpRingTarget(
      hits,
      currentPath,
      jumpRing,
      direction,
    );
    jumpRing.index = outcome.nextRing.index;
    jumpRing.lastPressTime = outcome.nextRing.lastPressTime;
    jumpRing.distinctPaths = outcome.nextRing.distinctPaths;
    const target = outcome.target;
    const ringPosition = outcome.ringPosition;
    const ringSize = outcome.ringSize;

    // Iter 46: routes through the shared `handleDispatchLedgerPath`
    // helper instead of a local inline dispatcher, so Jump Ring
    // and Activity Timeline clicks land on identical plumbing.
    await handleDispatchLedgerPath(target);

    // Iter 44: operation-preview toast. Post-dispatch because
    // the dispatch is the authoritative confirmation — we only
    // tell the user about a jump that actually happened. The
    // toast answers the question "why is this path here?" by
    // showing the ledger kind that ranked it + a relative
    // timestamp ("3m ago"). When the ring has more than one
    // slot, the action hint reveals the cycling gesture so
    // power users discover it without a tutorial. When the
    // destination's kind is unknown (legacy ledger rows), the
    // toast still fires but with a minimal "visited" verb so
    // the user always gets feedback. Reuses the existing
    // structured-error toast channel — no new notification
    // system, consistent with every other iter's feedback.
    const sourceHit = hits.find((h) => h.path === target);
    const kindInfo = sourceHit?.last_kind
      ? labelForKind(sourceHit.last_kind)
      : { label: "visited", tone: "" };
    const relativeTime = sourceHit?.last_seen
      ? formatRelativeTime(sourceHit.last_seen, undefined, "withSuffix")
      : null;
    const verb = kindInfo.label.charAt(0).toUpperCase() + kindInfo.label.slice(1);
    const destLabel = deriveLabel(target);
    useUIStore.getState().addStructuredError({
      what: `${verb} ${destLabel}`,
      why: relativeTime
        ? `${relativeTime} \u00b7 ${target}`
        : target,
      appDid:
        ringSize > 1
          ? `Jumped via Jump Ring (${ringPosition} of ${ringSize})`
          : "Jumped via Jump Ring",
      userAction:
        ringSize > 1
          ? "Press \u2318\u21E7L again to cycle forward, or \u2318\u21E7\u2325L to cycle back, within 2.5s"
          : "Perform more file operations to build a richer jump ring",
    });
  }, [handleDispatchLedgerPath]);

  // Quickflow Launchpad — eager-load saved automation rules once on
  // mount so the Command Palette can list manual-trigger rules from
  // the very first ⌘K, even when the user has never opened the
  // automation panel in this session. Cost: one bounded SQL query
  // through `list_automation_rules`. Idempotent — calling loadRules
  // a second time just refreshes the in-memory list.
  useEffect(() => {
    void loadAutomationRules();
  }, [loadAutomationRules]);

  // Iter 18: opportunistically refresh the drive list and free-space
  // numbers when the user returns to the window. Solves the "I plugged
  // in a USB while UFOP was in the background" gap from iter 17 — the
  // sidebar would otherwise wait up to 30s (iter-16 cache TTL) before
  // noticing the new mount. Same wire also keeps the status-bar
  // free-space honest when a long-running background transfer mutates
  // disk usage off-screen.
  //
  // Mirrors the pull-on-attention pattern in `activity-timeline-panel.tsx`
  // (search "onFocus") — focus + visibilitychange together cover both
  // app-switch (Cmd+Tab) and desktop-switch / minimise-restore on every
  // platform. Throttled to 5s so rapid focus toggling doesn't thrash
  // the IPC: a stale-enough cache invalidates, a fresh one does nothing.
  useEffect(() => {
    const STALE_AFTER_MS = 5_000;
    const onAttention = () => {
      if (document.visibilityState !== "visible") return;
      refreshDriveCacheIfStale(STALE_AFTER_MS);
    };
    window.addEventListener("focus", onAttention);
    document.addEventListener("visibilitychange", onAttention);
    return () => {
      window.removeEventListener("focus", onAttention);
      document.removeEventListener("visibilitychange", onAttention);
    };
  }, []);

  // Global keyboard shortcuts
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // ? (Shift+/) toggles the keyboard cheat sheet overlay. Pure
      // discoverability gesture: lists every palette entry with a
      // bound shortcut, grouped by category, auto-updated from the
      // same source of truth that drives the command palette.
      // Iter 17 counter-move to the accumulating shortcut load
      // from iters 10-16: instead of teaching one more thing,
      // surface everything that already exists.
      //
      // Defensive: ignore the keystroke when an input / textarea /
      // contenteditable has focus, otherwise typing `?` into any
      // text field (search, filter, AI prompt) would pop the
      // overlay. This matches the cheat-sheet convention used by
      // GitHub / Gmail / Slack / Linear / Notion.
      if (e.key === "?" && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const active = document.activeElement as HTMLElement | null;
        const tag = active?.tagName;
        const isEditable =
          tag === "INPUT" ||
          tag === "TEXTAREA" ||
          active?.isContentEditable === true;
        if (!isEditable) {
          e.preventDefault();
          setCheatSheetOpen((open) => !open);
          return;
        }
      }
      // Cmd+B to toggle sidebar
      if ((e.metaKey || e.ctrlKey) && e.key === "b") {
        e.preventDefault();
        toggleSidebar();
      }
      // Cmd+Z to undo
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        handleUndo();
      }
      // Cmd+Shift+Z to redo — symmetric Cmd+Z pair via the ledger-backed
      // `redo_last` IPC. Matches the universal redo gesture used by every
      // major editor and browser; pure additive (no change for users who
      // never undo).
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.key === "z" || e.key === "Z")) {
        e.preventDefault();
        handleRedo();
      }
      // Cmd+Shift+A to toggle AI panel (T-043)
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "A") {
        e.preventDefault();
        toggleAiPanel();
      }
      // Cmd+` to toggle terminal (T-046)
      if ((e.metaKey || e.ctrlKey) && e.key === "`") {
        e.preventDefault();
        toggleTerminalPanel();
      }
      // Cmd+Shift+Y to toggle Activity Timeline (unified ledger viewer)
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "Y") {
        e.preventDefault();
        toggleActivityTimeline();
      }
      // Cmd+Shift+O — Instant Jump. Opens the path picker that fuzzy-
      // searches every path the unified ledger has ever seen, across
      // connectors and sessions. Zero cognitive overload: invisible
      // until pressed.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "O") {
        e.preventDefault();
        usePathJumpStore.getState().open();
      }
      // Cmd+Shift+L — "Jump to Last Touched" Jump Ring. Iter 42
      // introduced the one-keystroke teleport; iter 43 upgrades
      // it into an alt-tab-style cycler. First press skips the
      // current active tab's path and goes to the newest
      // *different* context. Rapid re-press (<2500ms) cycles to
      // the next distinct recent path. After the cycle window
      // expires the ring resets. Zero new backend code — same
      // `ledger_recent_paths` IPC, now with limit=25 — and the
      // same shared `dispatchLedgerPath` helper routes archive /
      // text / preview / folder destinations exactly like
      // Instant Jump. No-op fallback: empty ledger shows a
      // structured-error toast; 25-hit filter wipe-out falls
      // through to the raw top hit.
      // Iter 45: split ⌘⇧L (forward) from ⌘⇧⌥L (reverse) on Alt.
      // Using `e.code === "KeyL"` for the reverse branch because
      // on Mac, Option+L produces a special character and `e.key`
      // would be something like "¬" instead of "L". `e.code` is
      // physical-layout-based and always reports "KeyL".
      if (
        (e.metaKey || e.ctrlKey) &&
        e.shiftKey &&
        !e.altKey &&
        e.key === "L"
      ) {
        e.preventDefault();
        void handleJumpLastTouched("forward");
      }
      if (
        (e.metaKey || e.ctrlKey) &&
        e.shiftKey &&
        e.altKey &&
        e.code === "KeyL"
      ) {
        e.preventDefault();
        void handleJumpLastTouched("reverse");
      }
      // Cmd+Shift+. to toggle hidden files (#85)
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === ".") {
        e.preventDefault();
        useFileManagerStore.getState().toggleHiddenFiles();
      }
      // Cmd+Shift+S to save workspace
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "S") {
        e.preventDefault();
        handleSaveWorkspace();
      }
      // Cmd+Shift+T — Reopen the most recently closed tab in the active
      // pane. Browser-muscle-memory shortcut: pops the active pane's
      // closed-tab ring buffer and revives the snapshot bit-exact (path,
      // label, full back/forward stack). No-op when the ring is empty,
      // so the shortcut is always safe to bind.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "T") {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        store.reopenClosedTab(store.activePaneIndex);
      }
      // Cmd+Shift+M — Stash the active pane's current selection into the
      // global stash ring. Pairs with Cmd+Shift+T (reopen tab) to give
      // the user a complete "where I left off" recovery story: closed
      // tabs come back via T, working selections come back via the
      // palette's "Recall Stash" entry. Surfaces a tiny toast on success
      // so the user gets feedback without opening the palette.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "M") {
        e.preventDefault();
        const stash = useFileManagerStore.getState().stashCurrentSelection();
        if (stash) {
          useUIStore.getState().addStructuredError({
            what: `Stashed ${stash.paths.length} file${stash.paths.length === 1 ? "" : "s"}`,
            why: `From ${stash.sourceDir}`,
            appDid:
              "Saved the current selection into the persistent stash ring — recall it any time from the Command Palette",
            userAction:
              "Open the Command Palette (\u2318K) and pick 'Recall Last Stash' to restore",
          });
        }
      }
      // Cmd+Shift+B — Toggle a bookmark on the active pane's current
      // directory. Reuses the same `favorites` ring the sidebar
      // drag-drop populates: zero new state surface, one new keystroke.
      // Surfaces a structured-error toast so the user gets feedback
      // (added/removed) without needing the sidebar to be visible.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "B") {
        e.preventDefault();
        const result = useFileManagerStore
          .getState()
          .toggleFavoriteForCurrentDir();
        if (result) {
          useUIStore.getState().addStructuredError({
            what: result.added
              ? `Bookmarked ${result.item.name}`
              : `Removed bookmark ${result.item.name}`,
            why: result.item.path,
            appDid: result.added
              ? "Added the active directory to your favorites — jump back any time with \u23181..\u23189 or the sidebar"
              : "Removed the directory from your favorites",
            userAction: result.added
              ? "Press \u23181..\u23189 to jump to the first nine bookmarks, or open the Command Palette (\u2318K)"
              : "Press \u2318\u21E7B again on the same folder to re-bookmark it",
          });
        }
      }
      // Cmd+Shift+E — Open the active pane's current path as a fresh
      // tab in the OTHER pane and focus it. Auto-exits single-pane
      // mode (revealing the second pane is part of the gesture).
      // Reuses `addTab` so the new tab's history bookkeeping matches
      // every other tab — DRY. Surfaces a structured-error toast on
      // success so the user gets immediate feedback.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "E") {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        const result = store.openInOtherPane(store.activePaneIndex);
        if (result) {
          useUIStore.getState().addStructuredError({
            what: `Opened ${result.label} in ${result.otherIndex === 0 ? "left" : "right"} pane`,
            why: result.path,
            appDid:
              "Mirrored the current location into the other pane as a fresh tab and focused it — no files were copied",
            userAction:
              "Use \u2318C / \u2318X to copy or move selected files into this side, or navigate independently from here",
          });
        }
      }
      // Cmd+Shift+R — Open the Batch Rename dialog on the active
      // pane's selection. The full rename engine (token
      // substitution, find/replace, regex, sequential numbering,
      // case transformation, undo, compatibility warnings) has
      // been living in the codebase as an orphaned component since
      // its original iteration \u2014 iter 25 surfaces it so users can
      // actually reach it. Guards against the empty-selection
      // case so the shortcut is a no-op when nothing is selected.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "R") {
        e.preventDefault();
        // Read the selection inline from the store rather than
        // via `getAllSelectedPaths` \u2014 this useEffect runs above
        // the helper's declaration site and a forward reference
        // trips the no-use-before-define lint rule.
        const store = useFileManagerStore.getState();
        const pane = store.getActivePane();
        const sel = store.selectedPaths[pane.id];
        if (sel && sel.length > 0) {
          setBatchRenameFiles([...sel]);
        }
      }
      // Cmd+, (Ctrl+, on Windows/Linux) \u2014 open the Settings
      // panel. The universal "settings/preferences" shortcut on
      // every desktop platform. Iter 26 surfaces the long-
      // orphaned `SettingsPanel` component (1354 LOC of
      // export/import, sync across installations, notification
      // preferences, privacy controls, log export) which was
      // fully-built with backend IPC but never imported by
      // `file-manager.tsx`. No shift modifier \u2014 matches macOS
      // System Settings and Cross-platform IDEs exactly.
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key === ",") {
        e.preventDefault();
        setSettingsOpen((open) => !open);
      }
      // Cmd+Shift+N \u2014 open the Connection Manager. Surfaces
      // the long-orphaned `ConnectionPanel` (1941 LOC managing
      // saved profiles, quick-connect, connection test, SSH
      // config import, third-party import, protocol list) for
      // the 17-connector protocol registry. All 17 Tauri
      // commands it needs are already registered in the Rust
      // backend \u2014 the component just had zero importers. N
      // mnemonic = "New connection" which matches the most
      // common entry path into the dialog.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "N") {
        e.preventDefault();
        setConnectionsOpen((open) => !open);
      }
      // Cmd+Shift+H \u2014 open the Sync Panel. Surfaces the
      // long-orphaned `SyncPanel` (1454 LOC) with sync-pair
      // health indicators, create-pair wizard, dry-run
      // preview, conflict resolution, reports, and rollback.
      // All 14 sync IPC commands are pre-registered in the
      // Rust backend \u2014 the component just had zero importers.
      // H mnemonic = "sync Health" which matches the panel's
      // primary UX (health dots + health scores).
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "H") {
        e.preventDefault();
        setSyncOpen((open) => !open);
      }
      // Cmd+Shift+J — Open the Transfer History panel (Journal).
      // Surfaces the long-orphaned `TransferHistoryPanel` (284 LOC)
      // that complements the transfer engine's journal/crash-recovery
      // layer. Shows every historical transfer with status, checksum
      // verification result, retry count, duration, and speed. Three
      // backend IPCs (search_transfer_history, export_transfer_history,
      // cleanup_transfer_history) were pre-registered; the component
      // just had zero importers until iter 30. Pure toggle — no
      // selection required, no context-dependent no-op.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "J") {
        e.preventDefault();
        setTransferHistoryOpen((open) => !open);
      }
      // Cmd+Shift+I — Open the Integrity Tools panel. Surfaces
      // the long-orphaned `IntegrityTools` (1071 LOC, T-063),
      // the LAST clean orphan with full backend wiring. Four
      // tabs: Checksums (MD5/SHA-1/SHA-256), Duplicates (fast
      // pass + hash verify), Tags & Labels, Smart Folders
      // (saved filter queries). Five pre-registered IPCs:
      // integrity_checksum, integrity_verify, compute_checksum,
      // integrity_find_duplicates, integrity_get_file_info.
      // "I for Integrity" mnemonic — the only still-free letter
      // that carried semantic meaning after the iter 25-30 arc.
      // Reads the active pane's current directory from the
      // store so the Duplicates / Smart Folder tabs have a
      // default base path. Toggle closes on second press.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "I") {
        e.preventDefault();
        setIntegrityToolsOpen((prev) => {
          if (prev !== null) return null;
          const fmStore = useFileManagerStore.getState();
          const pane = fmStore.panes[fmStore.activePaneIndex];
          const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
          return { dir: tab?.path ?? "/", files: [] };
        });
      }
      // Cmd+Shift+P \u2014 toggle the Preview panel on the first
      // selected file. Routes through the ui-store's pre-
      // existing `previewPanelOpen` slot (iter 29 surfaced
      // another half-wired orphan here) plus the new
      // `previewFilePath` + `openPreviewFor` / `closePreview`
      // actions iter 29 added. Press 1 with selection \u2192 opens
      // on that file. Press 2 (regardless of selection) \u2192
      // closes. Press 3 with a NEW selection \u2192 re-opens on
      // the new file. Reads selection inline from the store
      // to avoid TDZ with helpers declared below.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "P") {
        e.preventDefault();
        const uiStore = useUIStore.getState();
        if (uiStore.previewPanelOpen) {
          uiStore.closePreview();
        } else {
          const fmStore = useFileManagerStore.getState();
          const pane = fmStore.getActivePane();
          const sel = fmStore.selectedPaths[pane.id];
          if (sel && sel.length > 0) {
            uiStore.openPreviewFor(sel[0]);
          }
        }
      }
      // Cmd+Shift+G ("Get history") \u2014 toggle the Lineage Panel for the
      // first selected file. Symmetric mirror of Cmd+Shift+P: if the
      // panel is open, close it; otherwise open on the active selection
      // (or no-op when nothing is selected \u2014 silent because a toast on
      // every misfire of an unfocused shortcut would be noisy). Reads
      // selection inline from the store so the binding stays TDZ-safe
      // alongside the other helpers further down the file. Pairs with
      // the existing right-click "Show File History" + activity-dot
      // click \u2014 same backend command (`get_file_lineage`), now a
      // keyboard gesture too.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "G") {
        e.preventDefault();
        const lineageStore = useLineageStore.getState();
        if (lineageStore.pendingPath !== null) {
          lineageStore.close();
        } else {
          const fmStore = useFileManagerStore.getState();
          const pane = fmStore.getActivePane();
          const sel = fmStore.selectedPaths[pane.id];
          if (sel && sel.length > 0) {
            lineageStore.beginRequest(sel[0]);
          }
        }
      }
      // Cmd+Shift+C — Copy the active pane's selection (or the
      // current directory path when nothing is selected) to the
      // system clipboard, one path per line. Fixes the pre-existing
      // palette UX gap where `onCopyPath` silently dropped every
      // path but the first from a multi-file selection. Graceful
      // fallback ensures the gesture is always useful — never a
      // confusing no-op. Reuses `navigator.clipboard.writeText`
      // exactly like every other copy-path action in the palette.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "C") {
        e.preventDefault();
        const { paths, source } = useFileManagerStore
          .getState()
          .collectClipboardPaths();
        if (paths.length === 0) return;
        // Fire-and-forget: the clipboard write is async but the
        // toast should appear immediately. The surrounding
        // try/catch mirrors every other clipboard call in this
        // file — a clipboard permission denial is recoverable and
        // should never tear down the keyboard handler.
        void (async () => {
          try {
            await navigator.clipboard.writeText(paths.join("\n"));
            useUIStore.getState().addStructuredError({
              what:
                source === "selection"
                  ? `Copied ${paths.length} path${paths.length === 1 ? "" : "s"} to clipboard`
                  : "Copied current directory path to clipboard",
              why: paths.length === 1 ? paths[0] : `${paths.length} paths, newline-separated`,
              appDid:
                source === "selection"
                  ? "Wrote every selected file's absolute path to the system clipboard, one per line"
                  : "Wrote the active pane's current directory path to the system clipboard",
              userAction:
                "Paste into a terminal, script, ticket, or document with your system's usual paste shortcut",
            });
          } catch {
            useUIStore.getState().addStructuredError({
              what: "Clipboard write failed",
              why: "The browser / OS denied clipboard access",
              appDid:
                "Attempted navigator.clipboard.writeText but the permission was denied",
              userAction:
                "Focus the window, grant clipboard permission when prompted, and try again",
            });
          }
        })();
      }
      // Cmd+Shift+K — Duplicate the active tab as a fresh tab in
      // the SAME pane and focus it. Different from Cmd+Shift+E
      // (Open in Other Pane) which targets the inactive pane.
      // Use case: keep the current view visible while you navigate
      // a clone of it elsewhere — like browser "Duplicate Tab" but
      // bound to a single keystroke. Reuses `addTab` so the clone
      // gets its own fresh history bookkeeping (single source of
      // truth for tab birth — DRY). K mnemonic = "Klone" (D, the
      // more obvious choice, is taken by the pre-existing toggle-
      // single-pane-mode handler in dual-pane-layout.tsx).
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "K") {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        const result = store.duplicateActiveTab(store.activePaneIndex);
        if (result) {
          useUIStore.getState().addStructuredError({
            what: `Duplicated tab "${result.label}"`,
            why: result.path,
            appDid:
              "Created a fresh tab in the same pane pointing at the active tab's path and focused it. The clone has its own back/forward history starting from this point.",
            userAction:
              "Navigate the duplicate freely \u2014 the original tab stays put on whichever path it was viewing",
          });
        }
      }
      // Cmd+Shift+V — Mirror the active pane's view configuration
      // (viewMode, sortBy, sortAsc, groupBy) onto the OTHER pane in
      // a single keystroke. Replaces the 3-4 menu clicks needed to
      // align two panes for side-by-side comparison. Pairs with
      // iter 11 (Open in Other Pane), iter 12 (Swap), and iter 13
      // (Filter Echo) to complete the "make these two views
      // comparable" workflow. Silent no-op when both panes already
      // match (idempotent gesture, no useless toast).
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "V") {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        const result = store.mirrorViewToOtherPane(store.activePaneIndex);
        if (result) {
          const groupSuffix = result.groupBy
            ? `, grouped by ${result.groupBy}`
            : "";
          useUIStore.getState().addStructuredError({
            what: `Mirrored view to ${result.otherIndex === 0 ? "left" : "right"} pane`,
            why: `${result.viewMode} \u00b7 ${result.sortBy} ${result.sortAsc ? "asc" : "desc"}${groupSuffix}`,
            appDid:
              "Copied the active pane's view mode, sort column, sort direction, and grouping onto the other pane",
            userAction:
              "Use the view toggle / sort header on either side independently if you want them to diverge again",
          });
        }
      }
      // Cmd+Shift+F — Echo the active pane's current filter text onto
      // the OTHER pane. Pairs with iter 11 (Open in Other Pane) and
      // iter 12 (Swap Panes) to complete the dual-pane filter
      // workflow: mirror a folder side-by-side, type a filter on the
      // active side, hit Cmd+Shift+F to apply the same filter to the
      // other pane — no retyping. No-op (silent) when the source
      // filter is empty so the gesture never wipes the other side's
      // filter by accident.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "F") {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        const result = store.echoFilterToOtherPane(store.activePaneIndex);
        if (result) {
          useUIStore.getState().addStructuredError({
            what: `Echoed filter "${result.text}" to ${result.otherIndex === 0 ? "left" : "right"} pane`,
            why: "Both panes now filter for the same query",
            appDid:
              "Copied the active pane's filter text onto the other pane and revealed the second pane if it was hidden",
            userAction:
              "Clear the filter on either side with the X button or by selecting the text and pressing Backspace",
          });
        }
      }
      // Cmd+Shift+X — Swap the contents of the two panes. What was on
      // the left is now on the right and vice versa, taking selections,
      // tabs, and per-pane history along with them. Auto-exits single-
      // pane mode (the gesture only makes sense when both sides are
      // visible). Pairs with Cmd+Shift+E (Open in Other Pane, iter 11):
      // mirror a folder, realise the source/target sides are wrong,
      // hit Cmd+Shift+X to flip — no clicks, no menu hunting.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "X") {
        e.preventDefault();
        const result = useFileManagerStore.getState().swapPanes();
        if (result) {
          useUIStore.getState().addStructuredError({
            what: "Swapped left and right panes",
            why: result.activePath,
            appDid:
              "Flipped the two panes — every tab, selection, and per-pane history travelled with its content. Your focus stayed on the same physical side and now shows the other pane's location.",
            userAction:
              "Hit \u2318\u21E7X again to swap back, or press Tab to move focus to the other side",
          });
        }
      }
      // Cmd+1..Cmd+9 — Jump active pane to bookmark slot. No-op when
      // the slot is empty so the shortcut is always safe to bind.
      // Pure number keys (no shift) so the muscle memory matches
      // browser tab-switching.
      if (
        (e.metaKey || e.ctrlKey) &&
        !e.shiftKey &&
        !e.altKey &&
        e.key >= "1" &&
        e.key <= "9"
      ) {
        const index = Number(e.key) - 1;
        const store = useFileManagerStore.getState();
        if (store.favorites[index]) {
          e.preventDefault();
          store.jumpToFavoriteByIndex(store.activePaneIndex, index);
        }
      }
      // Cmd+[ or Alt+ArrowLeft to navigate back
      if (
        ((e.metaKey || e.ctrlKey) && e.key === "[") ||
        (e.altKey && e.key === "ArrowLeft")
      ) {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        store.navigateBack(store.activePaneIndex);
      }
      // Cmd+] or Alt+ArrowRight to navigate forward
      if (
        ((e.metaKey || e.ctrlKey) && e.key === "]") ||
        (e.altKey && e.key === "ArrowRight")
      ) {
        e.preventDefault();
        const store = useFileManagerStore.getState();
        store.navigateForward(store.activePaneIndex);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [toggleSidebar, undoStack, toggleAiPanel, toggleTerminalPanel, toggleActivityTimeline, handleUndo, handleRedo, handleSaveWorkspace, handleJumpLastTouched]);

  // "What happened while you were away?" — single mount-time ledger
  // summary call. Reads events added since the last session and shows a
  // dismissible toast if any activity ran (e.g. overnight automation
  // fires, background syncs). Backend atomically advances the
  // "last seen" marker. Zero value shown on fresh installs.
  const [sinceSummary, setSinceSummary] = useState<LedgerSinceSummary | null>(null);
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const summary = await tauriInvokeSafe<LedgerSinceSummary | null>(
        "ledger_since_last_seen",
        undefined,
        null,
      );
      if (!cancelled) setSinceSummary(summary);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Poll for editor file changes every 2 seconds (Issue #4: Editor File Watch)
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const changed = await tauriInvokeSafe<Array<{
          temp_path: string;
          remote_path: string;
          protocol: string;
          connection_id: string;
        }>>("check_watched_files", undefined, []);

        for (const file of changed) {
          // Auto-enqueue re-upload for changed files
          if (file.remote_path && file.temp_path) {
            await tauriInvoke("enqueue_transfer", {
              sourcePath: file.temp_path,
              destPath: file.remote_path,
              totalBytes: 0,
            }).catch(() => {});
          }
        }
      } catch {}
    }, 2000);

    return () => clearInterval(interval);
  }, []);

  // Helper to get the first selected path from the active pane
  const getFirstSelectedPath = useCallback((): string | null => {
    const store = useFileManagerStore.getState();
    const pane = store.getActivePane();
    const sel = store.selectedPaths[pane.id];
    if (sel && sel.length > 0) return sel[0];
    return null;
  }, []);

  // Iter 25: get ALL selected paths from the active pane. Needed
  // by Batch Rename which operates on the whole selection rather
  // than just the first entry. Parallels `getFirstSelectedPath`
  // so any future multi-select feature has a ready helper.
  const getAllSelectedPaths = useCallback((): string[] => {
    const store = useFileManagerStore.getState();
    const pane = store.getActivePane();
    const sel = store.selectedPaths[pane.id];
    return sel && sel.length > 0 ? [...sel] : [];
  }, []);

  // Quickflow Launchpad — convert each saved manual-trigger automation
  // rule into a CommandItem the user can fuzzy-search and fire from the
  // Command Palette (⌘K). Hidden when there are no manual rules so
  // first-time users see no extra entries; appears the moment the user
  // pins a ledger event or accepts a Quickflow suggestion.
  //
  // Stays out of the static `getDefaultCommands` call so the dynamic
  // rule list can re-render on every store update without invalidating
  // the rest of the palette's static commands. Memoized separately
  // against the rules array so we don't rebuild it on unrelated state
  // changes.
  const quickflowCommands = useMemo<CommandItem[]>(() => {
    const manual = automationRules.filter((r) => r.trigger.type === "manual");
    if (manual.length === 0) return [];
    return manual.map((rule) => ({
      id: `quickflow-run-${rule.id}`,
      label: rule.name,
      description: rule.enabled
        ? "Run this saved Quickflow now"
        : "Run this saved Quickflow now (currently disabled — will run as a one-shot)",
      icon: <Zap className="h-4 w-4" />,
      category: "Quickflows",
      keywords: ["run", "fire", "trigger", "quickflow", "automation", "rule", "replay"],
      action: () => {
        // Fire-and-forget; the structured-error toast surfaces the
        // outcome so the user gets feedback without blocking the
        // palette close. Rejection / failure is logged via the
        // automation store's existing error handler.
        void runAutomationRule(rule.id).then((log) => {
          useUIStore.getState().addStructuredError({
            what: log ? `Ran '${rule.name}'` : `'${rule.name}' failed`,
            why: log
              ? log.error_message ||
                `Affected ${log.files_affected.length} file${log.files_affected.length === 1 ? "" : "s"}`
              : "Backend rejected the run request",
            appDid: log
              ? "Dispatched the saved manual-trigger Quickflow via the existing automation engine"
              : "Could not invoke the rule",
            userAction: log
              ? "Press Cmd+Z to undo if this wasn't what you expected"
              : "Open the Automation panel and check the rule's last error",
          });
        });
      },
    }));
  }, [automationRules, runAutomationRule]);

  // Default commands for command palette
  const commands = useMemo(
    () => [
      ...quickflowCommands,
      ...getDefaultCommands({
        onToggleDualPane: () =>
          useFileManagerStore.getState().toggleSinglePaneMode(),
        onToggleSidebar: toggleSidebar,
        onRefresh: () => {
          window.dispatchEvent(new CustomEvent("ufop:refresh-directory"));
        },
        onUndo: handleUndo,
        onRedo: handleRedo,
        // Mirror of the Cmd+Shift+G keyboard binding — same toggle
        // semantics, same selection lookup, same store action. Routing
        // both surfaces through one inline closure keeps the palette
        // entry and the shortcut from drifting.
        onShowFileHistory: () => {
          const lineageStore = useLineageStore.getState();
          if (lineageStore.pendingPath !== null) {
            lineageStore.close();
            return;
          }
          const path = getFirstSelectedPath();
          if (path !== null) {
            lineageStore.beginRequest(path);
          }
        },
        onSetTheme: (theme) =>
          useUIStore.getState().setTheme(theme as "light" | "dark" | "system"),
        onSetViewMode: (mode) => {
          const store = useFileManagerStore.getState();
          store.setViewMode(store.activePaneIndex, mode as ViewMode);
        },
        onSaveWorkspace: handleSaveWorkspace,
        onLoadWorkspace: () => setWorkspaceMenuOpen(true),
        // Command-palette copy actions — previously each one silently
        // swallowed success and failure (try { ... } catch {}). They
        // now route through the shared `copyToClipboardWithToast`
        // helper so a user firing Copy Path via the palette gets the
        // same visible feedback as firing it via the context menu.
        // IPC-resolved variants explicitly distinguish a backend
        // rejection from a clipboard-write failure.
        onCopyPath: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          await copyToClipboardWithToast(path, "Copy Path");
        },
        onCopyRemotePath: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          let resolved: string;
          try {
            const result = await tauriInvoke<{ path: string }>("copy_remote_path", { path, remoteBase: null });
            resolved = result.path;
          } catch (err) {
            useUIStore.getState().addStructuredError({
              what: "Copy Remote Path failed",
              why: err instanceof Error ? err.message : "Backend rejected the request",
              appDid: "Did not write anything to the clipboard",
              userAction: "Confirm a remote base is configured for this connection",
            });
            return;
          }
          await copyToClipboardWithToast(resolved, "Copy Remote Path");
        },
        onCopyUrl: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          let resolved: string;
          try {
            const result = await tauriInvoke<{ path: string }>("copy_url", { path, protocol: "local", host: "localhost", port: null, username: null });
            resolved = result.path;
          } catch (err) {
            useUIStore.getState().addStructuredError({
              what: "Copy URL failed",
              why: err instanceof Error ? err.message : "Backend rejected the request",
              appDid: "Did not write anything to the clipboard",
              userAction: "Try the simple Copy Path action instead",
            });
            return;
          }
          await copyToClipboardWithToast(resolved, "Copy URL");
        },
        onCopyRelativePath: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          const store = useFileManagerStore.getState();
          const basePath = store.getActivePath(store.activePaneIndex);
          let resolved: string;
          try {
            const result = await tauriInvoke<{ path: string }>("copy_relative_path", { path, basePath });
            resolved = result.path;
          } catch (err) {
            useUIStore.getState().addStructuredError({
              what: "Copy Relative Path failed",
              why: err instanceof Error ? err.message : "Backend rejected the request",
              appDid: "Did not write anything to the clipboard",
              userAction: "Confirm the current pane has a valid base path",
            });
            return;
          }
          await copyToClipboardWithToast(resolved, "Copy Relative Path");
        },
        onOpenInEditor: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          const ext = path.split('.').pop() || "";
          try {
            const mapping = await tauriInvoke<{ app_path: string; args: string[] } | null>("resolve_editor", { extension: ext });
            await tauriInvoke("open_in_editor", { filePath: path, editorPath: mapping?.app_path || null, editorArgs: mapping?.args || null });
          } catch {}
        },
        onToggleAutomations: toggleAutomationPanel,
        onCreateAutomation: () => {
          if (!automationPanelOpen) toggleAutomationPanel();
          useAutomationStore.getState().openEditor();
        },
        onCreateSmartSpace: () => useSpacesStore.getState().openWizard(),
        onToggleActivityTimeline: toggleActivityTimeline,
        onReopenClosedTab: () => {
          const store = useFileManagerStore.getState();
          store.reopenClosedTab(store.activePaneIndex);
        },
        onStashSelection: () => {
          const stash = useFileManagerStore.getState().stashCurrentSelection();
          if (stash) {
            useUIStore.getState().addStructuredError({
              what: `Stashed ${stash.paths.length} file${stash.paths.length === 1 ? "" : "s"}`,
              why: `From ${stash.sourceDir}`,
              appDid:
                "Saved the current selection into the persistent stash ring",
              userAction:
                "Pick 'Recall Last Stash' from the Command Palette to restore",
            });
          } else {
            useUIStore.getState().addStructuredError({
              what: "Nothing to stash",
              why: "The active pane has no selected files",
              appDid: "Refused to create an empty stash entry",
              userAction:
                "Select one or more files in the active pane and try again",
            });
          }
        },
        onRecallLastStash: () => {
          const store = useFileManagerStore.getState();
          const last = store.selectionStashes?.[0];
          if (!last) {
            useUIStore.getState().addStructuredError({
              what: "No stashes to recall",
              why: "The selection stash ring is empty",
              appDid: "Refused to recall a non-existent stash",
              userAction:
                "Use 'Stash Selection' (\u2318\u21E7M) first to save a working set",
            });
            return;
          }
          store.recallStash(last.id, store.activePaneIndex);
        },
        onToggleBookmarkCurrentDir: () => {
          const result = useFileManagerStore
            .getState()
            .toggleFavoriteForCurrentDir();
          if (!result) return;
          useUIStore.getState().addStructuredError({
            what: result.added
              ? `Bookmarked ${result.item.name}`
              : `Removed bookmark ${result.item.name}`,
            why: result.item.path,
            appDid: result.added
              ? "Added the active directory to your favorites"
              : "Removed the directory from your favorites",
            userAction: result.added
              ? "Press \u23181..\u23189 to jump to the first nine bookmarks"
              : "Press \u2318\u21E7B again on the same folder to re-bookmark it",
          });
        },
        bookmarks: favorites,
        onJumpToBookmark: (index) => {
          const store = useFileManagerStore.getState();
          store.jumpToFavoriteByIndex(store.activePaneIndex, index);
        },
        onOpenInOtherPane: () => {
          const store = useFileManagerStore.getState();
          const result = store.openInOtherPane(store.activePaneIndex);
          if (!result) return;
          useUIStore.getState().addStructuredError({
            what: `Opened ${result.label} in ${result.otherIndex === 0 ? "left" : "right"} pane`,
            why: result.path,
            appDid:
              "Mirrored the current location into the other pane as a fresh tab",
            userAction:
              "Use \u2318C / \u2318X to copy or move selected files into this side",
          });
        },
        onSwapPanes: () => {
          const result = useFileManagerStore.getState().swapPanes();
          if (!result) return;
          useUIStore.getState().addStructuredError({
            what: "Swapped left and right panes",
            why: result.activePath,
            appDid:
              "Flipped the two panes \u2014 every tab, selection, and per-pane history travelled with its content",
            userAction:
              "Hit the same command again to swap back, or press Tab to move focus to the other side",
          });
        },
        onEchoFilterToOtherPane: () => {
          const store = useFileManagerStore.getState();
          const result = store.echoFilterToOtherPane(store.activePaneIndex);
          if (!result) return;
          useUIStore.getState().addStructuredError({
            what: `Echoed filter "${result.text}" to ${result.otherIndex === 0 ? "left" : "right"} pane`,
            why: "Both panes now filter for the same query",
            appDid:
              "Copied the active pane's filter text onto the other pane",
            userAction:
              "Clear the filter on either side with the X button",
          });
        },
        onCopyAllPaths: () => {
          const { paths, source } = useFileManagerStore
            .getState()
            .collectClipboardPaths();
          if (paths.length === 0) return;
          void (async () => {
            try {
              await navigator.clipboard.writeText(paths.join("\n"));
              useUIStore.getState().addStructuredError({
                what:
                  source === "selection"
                    ? `Copied ${paths.length} path${paths.length === 1 ? "" : "s"} to clipboard`
                    : "Copied current directory path to clipboard",
                why: paths.length === 1 ? paths[0] : `${paths.length} paths, newline-separated`,
                appDid:
                  source === "selection"
                    ? "Wrote every selected file's absolute path to the system clipboard, one per line"
                    : "Wrote the active pane's current directory path to the system clipboard",
                userAction:
                  "Paste into a terminal, script, ticket, or document",
              });
            } catch {
              useUIStore.getState().addStructuredError({
                what: "Clipboard write failed",
                why: "The browser / OS denied clipboard access",
                appDid:
                  "Attempted navigator.clipboard.writeText but the permission was denied",
                userAction: "Grant clipboard permission and try again",
              });
            }
          })();
        },
        onOpenSettings: () => {
          setSettingsOpen(true);
        },
        onOpenConnections: () => {
          setConnectionsOpen(true);
        },
        onOpenSync: () => {
          setSyncOpen(true);
        },
        onOpenTransferHistory: () => {
          setTransferHistoryOpen(true);
        },
        onOpenIntegrityTools: () => {
          // Snapshot the active pane's current directory the same
          // way the Cmd+Shift+I keyboard handler does, so palette-
          // initiated opens feel identical to keyboard-initiated
          // ones and the Duplicates / Smart Folder tabs have a
          // sensible default base path.
          const fmStore = useFileManagerStore.getState();
          const pane = fmStore.panes[fmStore.activePaneIndex];
          const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
          setIntegrityToolsOpen({ dir: tab?.path ?? "/", files: [] });
        },
        onJumpLastTouched: () => {
          void handleJumpLastTouched();
        },
        onOpenPreview: () => {
          const path = getFirstSelectedPath();
          if (path) {
            useUIStore.getState().openPreviewFor(path);
          } else {
            useUIStore.getState().addStructuredError({
              what: "Preview needs a selection",
              why: "Nothing is currently selected in the active pane",
              appDid:
                "Skipped opening the preview because there is no file to show",
              userAction:
                "Select a file in the active pane and try again (or press \u2318\u21E7P with a file selected)",
            });
          }
        },
        onBatchRename: () => {
          const paths = getAllSelectedPaths();
          if (paths.length === 0) {
            useUIStore.getState().addStructuredError({
              what: "Batch Rename needs a selection",
              why: "Nothing is currently selected in the active pane",
              appDid:
                "Skipped opening the dialog because there are no files to operate on",
              userAction:
                "Select one or more files in the active pane and try again",
            });
            return;
          }
          setBatchRenameFiles(paths);
        },
        onDuplicateActiveTab: () => {
          const store = useFileManagerStore.getState();
          const result = store.duplicateActiveTab(store.activePaneIndex);
          if (!result) return;
          useUIStore.getState().addStructuredError({
            what: `Duplicated tab "${result.label}"`,
            why: result.path,
            appDid:
              "Created a fresh tab in the same pane pointing at the active tab's path and focused it",
            userAction:
              "Navigate the duplicate freely \u2014 the original tab stays put",
          });
        },
        onMirrorViewToOtherPane: () => {
          const store = useFileManagerStore.getState();
          const result = store.mirrorViewToOtherPane(store.activePaneIndex);
          if (!result) return;
          const groupSuffix = result.groupBy
            ? `, grouped by ${result.groupBy}`
            : "";
          useUIStore.getState().addStructuredError({
            what: `Mirrored view to ${result.otherIndex === 0 ? "left" : "right"} pane`,
            why: `${result.viewMode} \u00b7 ${result.sortBy} ${result.sortAsc ? "asc" : "desc"}${groupSuffix}`,
            appDid:
              "Copied the active pane's view mode, sort, and grouping onto the other pane",
            userAction:
              "Use the view toggle / sort header on either side to diverge again",
          });
        },
      }),
    ],
    [quickflowCommands, toggleSidebar, handleUndo, handleRedo, handleSaveWorkspace, getFirstSelectedPath, getAllSelectedPaths, toggleAutomationPanel, automationPanelOpen, toggleActivityTimeline, favorites, handleJumpLastTouched],
  );

  return (
    <div className="flex h-full flex-col" data-testid="file-manager">
      {/* "What happened while you were away?" mount-time summary toast */}
      <SinceLastSeenToast summary={sinceSummary} onDismiss={() => setSinceSummary(null)} />
      {/* Toolbar */}
      <header
        className="relative flex items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-toolbar-bg)] px-4"
        style={{ height: "var(--toolbar-height)" }}
        role="toolbar"
        aria-label="Main toolbar"
        onContextMenu={(e) => { e.preventDefault(); toggleToolbarCustomizer(); }}
      >
        <div className="flex items-center gap-3">
          {/* In Simple Mode the outer SimpleModeWrapper provides sidebar toggle + title; suppress the duplicates here. */}
          {appMode !== "simple" && toolbarItems.includes("sidebar") && (
            <Button
              variant="ghost"
              size="icon"
              onClick={toggleSidebar}
              aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"}
              title="Toggle sidebar (Cmd+B)"
            >
              <PanelLeft className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          {appMode !== "simple" && (
            <>
              <FolderOpen className="h-5 w-5 text-[color:var(--color-primary)]" aria-hidden="true" />
              <h1 className="text-[length:var(--font-size-md)] font-semibold text-[color:var(--color-text)]">
                File Manager
              </h1>
              <Badge variant="secondary">{appMode}</Badge>
            </>
          )}
        </div>

        <div className="flex items-center gap-2">
          {toolbarItems.includes("view-modes") && <ViewModeSelector />}
          {toolbarItems.includes("hidden-files") && <HiddenFilesToggle />}
          {/* Sync Browsing toggle (#38) */}
          {toolbarItems.includes("sync-browse") && !singlePaneMode && (
            <Button
              variant={syncBrowsing ? "secondary" : "ghost"}
              size="icon"
              onClick={toggleSyncBrowsing}
              aria-label={syncBrowsing ? "Disable sync browsing" : "Enable sync browsing"}
              title={syncBrowsing ? "Sync Browse: ON" : "Sync Browse: OFF"}
              data-testid="toggle-sync-browse"
            >
              {syncBrowsing ? (
                <Link2 className="h-4 w-4" aria-hidden="true" />
              ) : (
                <Unlink2 className="h-4 w-4" aria-hidden="true" />
              )}
            </Button>
          )}
          {toolbarItems.includes("sync-browse") && syncBrowsing && !singlePaneMode && (
            <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-primary)] font-medium">Sync</span>
          )}
          {toolbarItems.includes("undo") && undoStack.length > 0 && (
            <Button
              variant="ghost"
              size="icon"
              onClick={handleUndo}
              aria-label="Undo last operation"
              title="Undo (Cmd+Z)"
            >
              <Undo2 className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          {/* Workspace menu */}
          <div className="relative">
            <Button
              variant={workspaceMenuOpen ? "secondary" : "ghost"}
              size="icon"
              onClick={() => setWorkspaceMenuOpen(!workspaceMenuOpen)}
              aria-label="Workspaces"
              aria-haspopup="menu"
              aria-expanded={workspaceMenuOpen}
              title="Workspaces"
              data-testid="workspace-menu-trigger"
            >
              <FolderDown className="h-4 w-4" aria-hidden="true" />
            </Button>
            {workspaceMenuOpen && (
              <div className="absolute right-0 top-full z-50 mt-1 w-64 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-lg overflow-hidden" role="menu">
                <div className="px-3 py-2 border-b border-[var(--color-border)] flex items-center justify-between">
                  <span className="text-xs font-semibold text-[color:var(--color-text)]">Workspaces</span>
                  <Button
                    variant="link"
                    size="sm"
                    className="text-xs gap-1 h-auto"
                    onClick={() => { handleSaveWorkspace(); }}
                    aria-label="Save current view as a workspace"
                  >
                    <Save className="h-3 w-3" aria-hidden="true" />
                    Save Current
                  </Button>
                </div>
                <div className="max-h-48 overflow-auto" role="none">
                  {workspaces.length === 0 ? (
                    <div className="px-3 py-4 text-xs text-center text-[color:var(--color-text-tertiary)]">
                      No saved workspaces yet.
                      <br />
                      Use Save Current or {"\u2318"}Shift+S
                    </div>
                  ) : (
                    workspaces.map((ws) => (
                      <div
                        key={ws.id}
                        className="flex items-center justify-between px-3 py-1.5 hover:bg-[var(--color-hover-bg)] group"
                        role="none"
                      >
                        <button
                          type="button"
                          role="menuitem"
                          onClick={() => handleLoadWorkspace(ws.id)}
                          className="flex-1 text-left text-xs text-[color:var(--color-text)] truncate focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
                          title={`Load "${ws.name}" (saved ${new Date(ws.savedAt).toLocaleDateString()})`}
                        >
                          {ws.name}
                        </button>
                        <button
                          type="button"
                          onClick={(e) => { e.stopPropagation(); handleDeleteWorkspace(ws.id); }}
                          className="text-xs text-[color:var(--color-error)] hover:opacity-80 opacity-0 group-hover:opacity-100 ml-2 focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
                          title={`Delete workspace "${ws.name}"`}
                          aria-label={`Delete workspace "${ws.name}"`}
                        >
                          Del
                        </button>
                      </div>
                    ))
                  )}
                </div>
              </div>
            )}
          </div>
          {toolbarItems.includes("automations") && (
            <Button
              variant={automationPanelOpen ? "secondary" : "ghost"}
              size="icon"
              onClick={toggleAutomationPanel}
              aria-label={automationPanelOpen ? "Close Quickflows" : "Open Quickflows"}
              title="Quickflows (Automations)"
              data-testid="toggle-automation-panel"
            >
              <Zap className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          {toolbarItems.includes("smart-spaces") && (
            <Button
              variant="ghost"
              size="icon"
              onClick={() => openSpacesWizard()}
              aria-label="Create Smart Space"
              title="Smart Spaces"
              data-testid="toggle-smart-spaces"
            >
              <Layers className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          {toolbarItems.includes("ai") && (
            <Button
              variant={aiPanelOpen ? "secondary" : "ghost"}
              size="icon"
              onClick={toggleAiPanel}
              aria-label={aiPanelOpen ? "Close AI assistant" : "Open AI assistant"}
              title="AI Assistant (Cmd+Shift+A)"
              data-testid="toggle-ai-panel"
            >
              <Bot className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          {toolbarItems.includes("terminal") && (
            <Button
              variant={terminalPanelOpen ? "secondary" : "ghost"}
              size="icon"
              onClick={() => {
                toggleTerminalPanel();
                const store = useTerminalStore.getState();
                if (!store.panelOpen && store.sessions.length === 0) {
                  createLocalSession();
                }
              }}
              aria-label={terminalPanelOpen ? "Close terminal" : "Open terminal"}
              title="Terminal (Cmd+`)"
              data-testid="toggle-terminal-panel"
            >
              <Terminal className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          {/* In Simple Mode the outer SimpleModeWrapper renders the ThemeSwitcher; suppress duplicate. */}
          {appMode !== "simple" && toolbarItems.includes("theme") && <ThemeSwitcher />}
        </div>

        {/* Toolbar Customizer (#14) */}
        {toolbarCustomizerOpen && (
          <div className="absolute right-0 top-full z-50 mt-1 p-3 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-lg min-w-[200px]">
            <h4 className="text-xs font-semibold mb-2 text-[color:var(--color-text)]">Customize Toolbar</h4>
            {ALL_TOOLBAR_ITEMS.map((item) => (
              <label key={item.id} className="flex items-center gap-2 py-1 text-xs text-[color:var(--color-text)] cursor-pointer">
                <input
                  type="checkbox"
                  checked={toolbarItems.includes(item.id)}
                  onChange={() => {
                    if (toolbarItems.includes(item.id)) {
                      setToolbarItems(toolbarItems.filter((i) => i !== item.id));
                    } else {
                      setToolbarItems([...toolbarItems, item.id]);
                    }
                  }}
                  className="h-3 w-3 rounded"
                />
                {item.label}
              </label>
            ))}
            <div className="mt-2 flex items-center gap-2">
              <button
                onClick={resetToolbar}
                className="flex-1 text-xs py-1 rounded border border-[var(--color-border)] text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
                title="Reset toolbar to default items"
              >
                Reset
              </button>
              <button
                onClick={toggleToolbarCustomizer}
                className="flex-1 text-xs py-1 rounded bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
              >
                Done
              </button>
            </div>
          </div>
        )}
      </header>

      {/* Main content area */}
      <div id="main-content" tabIndex={-1} className="flex flex-1 overflow-hidden">
        {sidebarOpen && (
          <aside
            className="border-r border-[var(--color-border)] shrink-0 overflow-hidden"
            style={{ width: sidebarWidth }}
            role="navigation"
            aria-label="File navigation sidebar"
          >
            <SidebarNav
              drives={sidebarDrives}
              onEjectDrive={async (mountPoint) => {
                if (!isTauriAvailable()) return;
                const driveLabel =
                  sidebarDrives.find((d) => d.path === mountPoint)?.name ??
                  mountPoint;
                try {
                  await tauriInvoke("eject_drive", { mountPoint });
                  // Bust the iter-16 drive cache so the just-ejected
                  // volume disappears from the sidebar and the
                  // status-bar free-space indicator immediately.
                  invalidateDriveCache();
                  useUIStore.getState().addStructuredError({
                    what: `Ejected ${driveLabel}`,
                    why: "The drive's mount table entry was unmounted",
                    appDid:
                      "Unmounted the volume so it's safe to disconnect physically",
                    userAction:
                      "Wait for the OS to spin the drive down, then unplug it",
                  });
                } catch (err) {
                  reportOperationFailure("Eject drive", err, {
                    userAction:
                      "Make sure no files on the drive are open in this or any other app",
                  });
                }
              }}
              onNavigate={(path) => {
                const store = useFileManagerStore.getState();
                const paneIndex = store.activePaneIndex;
                const activeTab = store.getActiveTab(paneIndex);
                if (activeTab) {
                  const label = path === "/" ? "Root" : path.split("/").filter(Boolean).pop() || "Root";
                  store.navigateTab(paneIndex, activeTab.id, path, label);
                  store.addRecentLocation(path);
                }
              }}
              onLoadChildren={isTauriAvailable() ? async (path: string) => {
                try {
                  return await tauriInvoke<FileEntryData[]>("list_directory", { path });
                } catch (err) {
                  console.error("Failed to load children for tree:", err);
                  return [];
                }
              } : undefined}
              onActivateSpace={(space) => {
                const store = useFileManagerStore.getState();
                // Navigate left pane to local path
                const pane0Tab = store.getActiveTab(0);
                if (pane0Tab) {
                  const label = space.local_path.split("/").filter(Boolean).pop() || space.local_path;
                  store.navigateTab(0, pane0Tab.id, space.local_path, label);
                }
                // If remote path, navigate right pane and ensure dual-pane mode
                if (space.remote_path) {
                  if (store.singlePaneMode) {
                    store.toggleSinglePaneMode();
                  }
                  const pane1Tab = store.getActiveTab(1);
                  if (pane1Tab) {
                    const label = space.remote_path.split("/").filter(Boolean).pop() || space.remote_path;
                    store.navigateTab(1, pane1Tab.id, space.remote_path, label);
                  }
                }
              }}
            />
          </aside>
        )}

        <div className="flex-1 flex flex-col overflow-hidden">
          <div className="flex-1 overflow-hidden">
            <DualPaneLayout
              renderPane={(paneIndex) => (
                <FilePane
                  paneIndex={paneIndex}
                  onContextMenu={setContextMenu}
                  onArchiveBrowse={setArchiveBrowserPath}
                  onTextEdit={(path, size) => setTextEditorFile({ path, size })}
                  onVerifyIntegrity={(files, dir) =>
                    setIntegrityToolsOpen({ dir, files })
                  }
                  onCompress={(files) => setCreateArchiveSources(files)}
                />
              )}
            />
          </div>
          <TerminalPanel />
        </div>

        {/* Lazy-mount: only render side panels when their toggle is on, so a fresh window
            ships with just the file canvas + sidebar instead of four hidden panels. */}
        {automationPanelOpen && <AutomationPanel />}
        {activityTimelineOpen && (
          <ActivityTimelinePanel onDispatchPath={handleDispatchLedgerPath} />
        )}
        {lineagePanelOpen && <LineagePanel />}
        {aiPanelOpen && <AiPanel />}
      </div>

      {/* Smart Space Wizard */}
      {spacesWizardOpen && <SmartSpaceWizard onClose={closeSpacesWizard} />}

      {/* Instant Jump — Cmd+Shift+O universal path recall.
          Iter 37: wired to the iter-31/32 contextual-routing
          infrastructure so path hits that are FILES (which the
          ledger started recording for real in iter 35 via the
          edit_text kind) open in the in-app editor, in the
          archive browser, or reveal in parent directory,
          instead of silently failing to navigate INTO a file. */}
      <PathJumpDialog
        isArchivePath={isArchivePath}
        isPlainTextPath={isPlainTextPath}
        isPreviewablePath={isPreviewablePath}
        onOpenArchive={(path) => setArchiveBrowserPath(path)}
        onOpenFile={(path, size) => setTextEditorFile({ path, size })}
        onOpenPreview={(path) => useUIStore.getState().openPreviewFor(path)}
      />

      {/* Safety Interlock — context-aware confirmation for anomalous ops */}
      <SafetyInterlockDialog />

      <footer
        className="relative flex items-center justify-between border-t border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-4"
        style={{ height: "var(--status-bar-height)" }}
        role="contentinfo"
        aria-label="Status bar"
      >
        <StatusBarContent />
      </footer>

      <ContextMenu
        items={contextMenu?.items || []}
        position={contextMenu?.position || null}
        onClose={() => setContextMenu(null)}
      />

      <CommandPalette
        commands={commands}
        isOpen={commandPaletteOpen}
        onClose={() => setCommandPaletteOpen(false)}
        onExecutionResult={handleIntentResult}
      />

      <KeyboardCheatSheet
        commands={commands}
        open={cheatSheetOpen}
        onClose={() => setCheatSheetOpen(false)}
      />

      {/* Iter 25: Batch Rename dialog. Renders as a full-screen
          modal overlay ONLY when `batchRenameFiles` is non-null
          (zero runtime cost when closed). The onComplete
          callback dispatches a path-aware refresh via the
          iter-18 helper so every pane viewing the affected
          parent directories auto-refreshes \u2014 the renamed files
          appear under their new names without the user
          clicking refresh. */}
      {/* Iter 29: Preview Pane modal. PreviewPane's own API
          takes `filePath`, `isOpen`, `onToggle`. The visibility
          and file path live in ui-store (iter 29 completed the
          pre-existing half-wired `previewPanelOpen` +
          `togglePreviewPanel` + `previewPanelWidth` orphan by
          adding `previewFilePath` + `openPreviewFor` +
          `closePreview`). Backdrop click and the component's
          own toggle both route through `closePreview`. */}
      {previewPanelOpen && previewFilePath && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="File Preview"
          data-testid="preview-backdrop"
          onClick={() => useUIStore.getState().closePreview()}
        >
          <div
            className="w-[min(720px,92vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <PreviewPane
              filePath={previewFilePath}
              isOpen={true}
              onToggle={() => useUIStore.getState().closePreview()}
            />
          </div>
        </div>
      )}

      {/* Iter 28: Sync Panel modal. Hosts the long-orphaned
          `SyncPanel` (1454 LOC) with health indicators,
          create-pair wizard, dry-run preview, conflict
          resolution, reports, and rollback. All 14 sync IPC
          commands were pre-registered in the Rust backend;
          iter 28 just makes the UI reachable. Same
          backdrop-click-to-close + stopPropagation pattern
          as every other iter-17/25/26/27 overlay. */}
      {syncOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Sync Panel"
          data-testid="sync-backdrop"
          onClick={() => setSyncOpen(false)}
        >
          <div
            className="w-[min(1100px,95vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="relative h-full">
              <button
                className="absolute top-3 right-4 z-10 h-7 w-7 flex items-center justify-center rounded text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover)]"
                onClick={() => setSyncOpen(false)}
                aria-label="Close Sync Panel"
              >
                <svg
                  className="w-4 h-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </button>
              <SyncPanel />
            </div>
          </div>
        </div>
      )}

      {/* Iter 30: Transfer History (Journal) modal. Hosts the
          long-orphaned `TransferHistoryPanel` (284 LOC) that
          complements the transfer engine's journal + crash-
          recovery layer. Shows every historical transfer with
          status, checksum verification, retry count, duration,
          and speed; supports CSV/JSON export and retention
          cleanup. Three backend IPCs were pre-registered; iter
          30 just makes the UI reachable via Cmd+Shift+J and a
          palette entry. Same backdrop-click-to-close +
          stopPropagation pattern as every other iter-17/25-29
          overlay. Sized to match the other data-table modals
          (Connection Manager, Sync Panel) for visual
          consistency when the user bounces between them. */}
      {transferHistoryOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Transfer History"
          data-testid="transfer-history-backdrop"
          onClick={() => setTransferHistoryOpen(false)}
        >
          <div
            className="w-[min(1100px,95vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="relative h-full">
              <button
                className="absolute top-3 right-4 z-10 h-7 w-7 flex items-center justify-center rounded text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover)]"
                onClick={() => setTransferHistoryOpen(false)}
                aria-label="Close Transfer History"
              >
                <svg
                  className="w-4 h-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </button>
              <TransferHistoryPanel />
            </div>
          </div>
        </div>
      )}

      {/* Iter 36: Integrity Tools modal. Hosts the last clean
          orphan (`IntegrityTools`, 1071 LOC, T-063) — four tabs
          covering checksums, duplicate detection, tags/labels,
          and smart folders. All five backend IPCs were
          pre-registered; only wiring was missing. Cmd+Shift+I
          opens with the active pane's directory snapshotted as
          `currentDirectory` so the Duplicates and Smart Folder
          scans have a default base path. Selection is passed
          as an empty array for now — the user can drive
          checksums via the Duplicates tab's scan output, and a
          future context-menu entry can thread in the active
          pane's selection for direct ad-hoc hashing. Same
          backdrop-click-to-close pattern as every other
          iter-25..30 overlay; sized to match the other
          data-table modals for visual consistency. */}
      {integrityToolsOpen !== null && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Integrity Tools"
          data-testid="integrity-tools-backdrop"
          onClick={() => setIntegrityToolsOpen(null)}
        >
          <div
            className="w-[min(1100px,95vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <IntegrityTools
              selectedFiles={integrityToolsOpen.files}
              currentDirectory={integrityToolsOpen.dir}
              onClose={() => setIntegrityToolsOpen(null)}
            />
          </div>
        </div>
      )}

      {/* Iter 31: Archive Browser modal. Opens the long-orphaned
          `ArchiveBrowser` (680 LOC, T-062) as a virtual-folder
          view over the archive the user double-clicked. Unlike
          iters 25-30 this modal is purely CONTEXTUAL — it has no
          global keyboard shortcut and no palette entry, because
          the natural entry point (double-clicking an archive file)
          was the component's original design intent. Closing
          clears `archiveBrowserPath` which unmounts the component,
          so the in-component state (current_dir, entries, etc.)
          resets cleanly on each invocation. Same backdrop-click-
          to-close + stopPropagation pattern as every other
          iter-17/25-30 overlay; sized to match the other data-
          table modals for visual consistency. */}
      {archiveBrowserPath && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Archive Browser"
          data-testid="archive-browser-backdrop"
          onClick={() => setArchiveBrowserPath(null)}
        >
          <div
            className="w-[min(1100px,95vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <ArchiveBrowser
              archivePath={archiveBrowserPath}
              onClose={() => setArchiveBrowserPath(null)}
            />
          </div>
        </div>
      )}

      {/* Iter 41: Create Archive dialog. The component itself already
          owns its overlay chrome (fixed full-screen wrapper inside
          `CreateArchiveDialog`), so we only gate mounting on the
          non-null sources state. `onComplete` fires the standard
          path-aware refresh so any pane viewing the destination
          directory immediately shows the new archive file. */}
      {createArchiveSources !== null && (
        <CreateArchiveDialog
          sourcePaths={createArchiveSources}
          isOpen={true}
          onClose={() => setCreateArchiveSources(null)}
          onComplete={(result) => {
            if (result.success && result.archive_path) {
              const parent = result.archive_path.substring(
                0,
                result.archive_path.lastIndexOf("/"),
              );
              if (parent) dispatchRefresh([parent]);
            }
          }}
        />
      )}

      {/* Iter 32: Text Editor modal. Hosts the long-orphaned
          `TextEditor` (234 LOC) via `TextEditorModal` which
          bridges the editor's pure-UI contract to the two IPCs
          added to `fs_commands.rs` in iter 32. Same contextual
          entry point as the ArchiveBrowser modal above: only
          mounted when the user double-clicks a plain-text or
          config file (from the `isPlainText` whitelist in
          `handleOpen`). Closing clears `textEditorFile` which
          unmounts the modal and drops the loaded content from
          memory. The backdrop does NOT close on click because an
          accidental click on the backdrop would discard
          unsaved edits; users must use the editor's explicit
          close button (which in a future iteration should
          prompt when `isDirty`). */}
      {textEditorFile && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Text Editor"
          data-testid="text-editor-backdrop"
        >
          <div className="w-[min(1100px,95vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]">
            <TextEditorModal
              filePath={textEditorFile.path}
              fileSize={textEditorFile.size}
              onClose={() => setTextEditorFile(null)}
            />
          </div>
        </div>
      )}

      {/* Iter 27: Connection Manager modal. Hosts the
          long-orphaned `ConnectionPanel` (1941 LOC) which
          manages saved profiles across the 17 connector
          protocols. Backend IPC (17 commands) was already
          wired; iter 27 just makes the UI reachable. Same
          backdrop-click-to-close + stopPropagation pattern as
          every other iter-17/25/26 overlay. */}
      {connectionsOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Connection Manager"
          data-testid="connections-backdrop"
          onClick={() => setConnectionsOpen(false)}
        >
          <div
            className="w-[min(1100px,95vw)] h-[min(800px,92vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="relative h-full">
              <button
                className="absolute top-3 right-4 z-10 h-7 w-7 flex items-center justify-center rounded text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover)]"
                onClick={() => setConnectionsOpen(false)}
                aria-label="Close Connection Manager"
              >
                <svg
                  className="w-4 h-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </button>
              <ConnectionPanel />
            </div>
          </div>
        </div>
      )}

      {/* Iter 26: Settings panel modal. Follows the same
          backdrop-click-to-close + stopPropagation pattern as
          iter 25's Batch Rename and iter 17's Keyboard Cheat
          Sheet overlays. The panel itself was fully built as
          a 1354-LOC orphan with 9+ backend commands already
          registered \u2014 iter 26 just wires it to the user-facing
          surface via Cmd+, and the command palette. */}
      {settingsOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Settings"
          data-testid="settings-backdrop"
          onClick={() => setSettingsOpen(false)}
        >
          <div
            className="w-[min(900px,95vw)] h-[min(720px,90vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="relative h-full">
              <button
                className="absolute top-3 right-4 z-10 h-7 w-7 flex items-center justify-center rounded text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover)]"
                onClick={() => setSettingsOpen(false)}
                aria-label="Close settings"
              >
                <svg
                  className="w-4 h-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </button>
              <SettingsPanel />
            </div>
          </div>
        </div>
      )}

      {batchRenameFiles && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Batch rename files"
          data-testid="batch-rename-backdrop"
          onClick={() => setBatchRenameFiles(null)}
        >
          <div
            className="w-[min(900px,95vw)] h-[min(720px,90vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)]"
            onClick={(e) => e.stopPropagation()}
          >
            <BatchRename
              files={batchRenameFiles}
              onClose={() => setBatchRenameFiles(null)}
              onComplete={(result) => {
                // Derive affected parent directories via the
                // pure helper so iter-18's `dispatchRefresh`
                // reaches every pane viewing them. The new
                // names land in the same parents, so a single
                // parent-dir dispatch covers both sides.
                if (result.success_count > 0 && batchRenameFiles) {
                  dispatchRefresh(parentDirectoriesOf(batchRenameFiles));
                }
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Hidden Files Toggle Button (#85)
// ──────────────────────────────────────────────

function HiddenFilesToggle() {
  const showHiddenFiles = useFileManagerStore((s) => s.showHiddenFiles);
  const toggleHiddenFiles = useFileManagerStore((s) => s.toggleHiddenFiles);

  return (
    <Button
      variant={showHiddenFiles ? "secondary" : "ghost"}
      size="icon"
      onClick={toggleHiddenFiles}
      aria-label={showHiddenFiles ? "Hide hidden files" : "Show hidden files"}
      title={`${showHiddenFiles ? "Hide" : "Show"} hidden files (Cmd+Shift+.)`}
      data-testid="toggle-hidden-files"
    >
      {showHiddenFiles ? (
        <Eye className="h-4 w-4" aria-hidden="true" />
      ) : (
        <EyeOff className="h-4 w-4" aria-hidden="true" />
      )}
    </Button>
  );
}

// ──────────────────────────────────────────────
// FilePane - single pane with tabs, breadcrumbs, file list
// ──────────────────────────────────────────────

interface FilePaneProps {
  paneIndex: 0 | 1;
  onContextMenu: (menu: { position: { x: number; y: number }; items: ContextMenuItem[] }) => void;
  /** Iter 31: lifts the archive-file "open as virtual folder"
   *  decision up to the top-level FileManager so the modal lives
   *  once across both panes. `handleOpen` calls this when the
   *  double-clicked file has an archive extension; the parent
   *  stashes the path and mounts the `ArchiveBrowser` overlay. */
  onArchiveBrowse: (path: string) => void;
  /** Iter 32: same lift pattern for the in-app text editor.
   *  Called with the file path and its size so the parent can
   *  mount `TextEditorModal` and the editor's 1 MB guard can
   *  render without a metadata round-trip. */
  onTextEdit: (path: string, size: number) => void;
  /** Iter 39: open Integrity Tools with the right-clicked
   *  files and the active pane's directory. Called from the
   *  "Verify Integrity…" context-menu entry so the Checksums
   *  tab is pre-populated with the user's current selection
   *  (or the right-clicked entry when nothing is selected). */
  onVerifyIntegrity: (files: string[], dir: string) => void;
  /** Iter 41: open the `CreateArchiveDialog` (long-orphaned tail of
   *  the iter 31 archive integration) pre-filled with the right-
   *  clicked selection. Singleton modal lives at FileManager scope so
   *  both panes share one dialog instance — same pattern as
   *  `onArchiveBrowse` / `onVerifyIntegrity` above. */
  onCompress: (files: string[]) => void;
}

function FilePane({
  paneIndex,
  onContextMenu,
  onArchiveBrowse,
  onTextEdit,
  onVerifyIntegrity,
  onCompress,
}: FilePaneProps) {
  const pane = useFileManagerStore((s) => s.panes[paneIndex]);
  const navigateTab = useFileManagerStore((s) => s.navigateTab);
  const addRecentLocation = useFileManagerStore((s) => s.addRecentLocation);
  const pushUndo = useFileManagerStore((s) => s.pushUndo);
  const showHiddenFiles = useFileManagerStore((s) => s.showHiddenFiles);
  const setGroupBy = useFileManagerStore((s) => s.setGroupBy);
  const appModePane = useUIStore((s) => s.appMode);

  const activeTab = pane.tabs.find((t) => t.id === pane.activeTabId);
  const currentPath = activeTab?.path || "/";

  const [files, setFiles] = useState<FileEntryData[]>([]);
  const [focusedIndex, setFocusedIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Path Recall — when loadDirectory fails because the path no
  // longer exists, the existing fallback silently substitutes
  // demo content. That's confusing: the user can't tell whether
  // they're seeing real data or stale. We layer the unified
  // ledger's lineage engine ON TOP of the existing fallback to
  // answer the universal "where did my file go?" question.
  //
  // The state is `null` whenever there is nothing useful to say
  // (no error, or no events in the ledger). The pure helper
  // `inferPathRecall` decides — see `@/lib/path-recall`.
  const [pathRecall, setPathRecall] = useState<PathRecallInfo | null>(null);

  // Feature 2: Git status (#46)
  const [gitStatus, setGitStatus] = useState<Record<string, string>>({});
  const [isGitRepo, setIsGitRepo] = useState(false);

  // Ledger-backed per-file activity dots. Fetches once per directory
  // change via a single `ledger_directory_activity` IPC call — no
  // polling, no background work, no per-row round-trips. Keyed by
  // full path; missing keys render nothing in the file list.
  const activityMap = useDirectoryActivity(currentPath);
  // Stale Files Filter — paths in the current directory that the unified
  // ledger HAS seen activity on within the staleness window (default 30
  // days). The set is intentionally the *complement* of "stale" so the
  // membership test is O(1) and the consumer flips it: stale = not in set.
  // The `loaded` flag is critical: until the backend answers, we cannot
  // distinguish "no recent activity" from "haven't asked yet", and
  // counting against an empty set would flag every file as stale during
  // the brief loading window. Always gate on `loaded` before flipping.
  const { touched: recentlyTouchedSet, loaded: staleLoaded } =
    useRecentlyTouchedSet(currentPath);
  // Per-pane toggle state for the Stale Files Filter chip. Local state
  // (not store-persisted) because staleness is a transient cleanup mode,
  // not a navigation preference — restoring it across sessions would
  // confuse users who don't remember enabling it.
  const [staleOnly, setStaleOnly] = useState(false);

  // Feature 5: Editable comments (#82)
  const [infoDialog, setInfoDialog] = useState<{
    path: string;
    name: string;
    metadata: Record<string, any> | null;
    comment: string;
  } | null>(null);

  // Feature 2: Folder size calculation (#83) — pane-local cache of
  // computed recursive folder sizes. Keyed by absolute folder path.
  // Populated by the "Calculate Size" context-menu action via
  // `handleCalculateSize`; consumed by `<FileList folderSizes={...}>`
  // to surface the cached value in the Size column once computed.
  // Lives in pane state because freshly listed directories don't
  // pre-compute size (that would walk the whole tree on every
  // navigation — too expensive); the user opts in per folder.
  const [folderSizes, setFolderSizes] = useState<Record<string, number>>({});

  // Feature 3: Quick Select (#71)
  const [quickSelectOpen, setQuickSelectOpen] = useState(false);
  const [quickSelectPattern, setQuickSelectPattern] = useState("");

  // Compare files state (#47) — populated by `handleCompareFiles`
  // when the user picks "Compare Selected" with two files; consumed
  // by the `<CompareFilesModal>` mounted at the end of this pane's
  // JSX tree. Each pane owns its own compare state, mirroring how
  // every other pane-local feature (filter text, focused index,
  // gitStatus) works.
  const [compareData, setCompareData] = useState<CompareData | null>(null);

  // Feature 5: Per-folder view defaults debounce timer (#13)
  const folderViewSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Load directory contents ──

  // Cancellation token for the in-flight Path Recall lineage fetch.
  // Each loadDirectory call increments this; a lineage resolution
  // whose token no longer matches is discarded, so rapid navigation
  // can never show recall info for a path the user already left.
  const pathRecallTokenRef = useRef(0);

  const loadDirectory = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    setPathRecall(null);
    // Bump the recall token before any async work so any earlier
    // in-flight lineage fetch becomes a no-op when it resolves.
    const token = ++pathRecallTokenRef.current;
    try {
      if (isTauriAvailable()) {
        const entries = await tauriInvoke<FileEntryData[]>("list_directory", { path });
        setFiles(entries);
      } else {
        setFiles(generateDemoFiles(path));
      }
    } catch (err) {
      console.error("Failed to list directory:", err);
      setError(String(err));
      setFiles(generateDemoFiles(path));
      // Path Recall — ask the ledger whether it has any history for
      // this path so we can show the user where it went. Fail-soft:
      // any error during the lineage fetch is silently swallowed,
      // leaving pathRecall=null and the user with the existing
      // demo-files behaviour. The IPC already exists and is read-
      // only (one SQL scan), so this adds no operational cost.
      if (isTauriAvailable()) {
        try {
          const lineage = await tauriInvokeSafe<FileLineage | null>(
            "get_file_lineage",
            { path },
            null,
          );
          // Stale-result guard: another loadDirectory call may have
          // started while we were awaiting. Compare the captured
          // token against the latest ref value — only the most
          // recent loader is allowed to set pathRecall.
          if (token === pathRecallTokenRef.current) {
            setPathRecall(inferPathRecall(lineage, path));
          }
        } catch {
          /* lineage best-effort; keep demo-files fallback */
        }
      }
    } finally {
      setLoading(false);
      setFocusedIndex(0);
    }
  }, []);

  useEffect(() => {
    loadDirectory(currentPath);
  }, [currentPath, loadDirectory]);

  // Feature 2: Detect git repo after files load (#46)
  useEffect(() => {
    const hasGitDir = files.some((f) => f.name === ".git" && f.is_dir);
    setIsGitRepo(hasGitDir);
    if (hasGitDir) {
      // When git backend is available, populate real status here.
      setGitStatus({});
    } else {
      setGitStatus({});
    }
  }, [files]);

  // Feature 5: Restore per-folder view defaults when path changes (#13)
  useEffect(() => {
    const store = useFileManagerStore.getState();
    const defaults = store.getFolderViewDefault(currentPath);
    if (defaults) {
      store.setViewMode(paneIndex, defaults.viewMode);
      store.setSorting(paneIndex, defaults.sortBy, defaults.sortAsc);
    }
  }, [currentPath, paneIndex]);

  // Feature 5: Auto-save per-folder view defaults on view/sort change (#13)
  useEffect(() => {
    if (folderViewSaveTimerRef.current) {
      clearTimeout(folderViewSaveTimerRef.current);
    }
    folderViewSaveTimerRef.current = setTimeout(() => {
      useFileManagerStore.getState().saveFolderViewDefault(currentPath, {
        viewMode: pane.viewMode,
        sortBy: pane.sortBy,
        sortAsc: pane.sortAsc,
      });
    }, 1000);
    return () => {
      if (folderViewSaveTimerRef.current) {
        clearTimeout(folderViewSaveTimerRef.current);
      }
    };
  }, [pane.viewMode, pane.sortBy, pane.sortAsc, currentPath]);

  // Listen for path-aware refresh events (iter 18). Each pane
  // checks whether the broadcasted affected paths intersect its
  // current view before reloading — unrelated panes skip the
  // re-fetch. Undefined / missing detail falls back to the
  // pre-iter-18 behaviour ("refresh always") so legacy dispatchers
  // (e.g. the palette Refresh action) keep working.
  useEffect(() => {
    const handler = (e: Event) => {
      const ce = e as CustomEvent<RefreshEventDetail>;
      const affected = ce.detail?.affectedPaths ?? [];
      if (shouldRefreshPane(currentPath, affected)) {
        loadDirectory(currentPath);
      }
    };
    window.addEventListener(REFRESH_EVENT, handler);
    return () => window.removeEventListener(REFRESH_EVENT, handler);
  }, [currentPath, loadDirectory]);

  // Apply filter
  const filteredFiles = useMemo(
    () => filterFiles(files, pane.filterText),
    [files, pane.filterText],
  );

  // Stale Files Filter — when active, restrict to non-directory files
  // that have NO ledger activity within the staleness window. The
  // recently-touched set is the *positive* signal; we flip it here.
  // Directories are always shown so navigation isn't crippled. Also
  // gated on `staleLoaded` so a stale toggle that survives a directory
  // change can't briefly flag everything as stale before the new
  // directory's data arrives.
  const staleFilteredFiles = useMemo(() => {
    if (!staleOnly || !staleLoaded) return filteredFiles;
    return filteredFiles.filter(
      (f) => f.is_dir || !recentlyTouchedSet.has(f.path),
    );
  }, [filteredFiles, staleOnly, staleLoaded, recentlyTouchedSet]);

  // Stale count — surfaced on the FilterBar chip so the user knows
  // *before* toggling whether there's anything stale to clean up. Same
  // predicate as the filter, computed against the unfiltered list so
  // the badge reflects the entire folder, not the active filter view.
  // Returns `undefined` until the recently-touched set has loaded so
  // the FilterBar suppresses the chip during the brief loading window
  // and outside Tauri (browser preview), avoiding a false-positive
  // flash that would flag every file as stale.
  const staleCount = useMemo<number | undefined>(() => {
    if (!staleLoaded) return undefined;
    let n = 0;
    for (const f of files) {
      if (!f.is_dir && !recentlyTouchedSet.has(f.path)) n++;
    }
    return n;
  }, [files, recentlyTouchedSet, staleLoaded]);

  // Smart Archive — one-click bulk move of every currently-stale file
  // in this directory into a dated `.archive/{YYYY-MM}/` subfolder.
  // Completes the cleanup story started by the Stale Filter chip:
  // see → click → done. Reuses the same safety + undo plumbing as
  // every other move in the app, so:
  //
  //   1. assessBeforeExecute pops the existing safety dialog when the
  //      operation is large enough to warrant confirmation,
  //   2. move_files (existing fs IPC) creates the .archive subfolder
  //      automatically and handles per-file conflicts,
  //   3. pushUndo records the operation so Universal Undo can reverse
  //      the entire archive in one shot,
  //   4. loadDirectory refreshes the pane so the chip count drops to
  //      zero (or to the new lower number) immediately after the move.
  //
  // The function is a no-op when there are no stale files, when we're
  // already inside an .archive directory (avoid recursive nesting), or
  // when running outside Tauri.
  const handleArchiveStale = useCallback(async () => {
    if (!staleLoaded) return;
    if (!isTauriAvailable()) {
      console.log("Smart Archive (demo mode): no-op outside Tauri");
      return;
    }
    // Refuse to nest .archive folders. The check is a simple substring
    // because the convention is `.archive/YYYY-MM/`; checking the
    // current path's tail covers both Unix and Windows separators.
    if (currentPath.includes("/.archive/") || currentPath.endsWith("/.archive")) {
      console.log("Smart Archive: already inside .archive — skipping");
      return;
    }
    // Gather every stale file in the current directory. Same predicate
    // as `staleFilteredFiles` so the bulk action operates on exactly
    // what the user sees with the chip active.
    const stalePaths: string[] = [];
    for (const f of files) {
      if (!f.is_dir && !recentlyTouchedSet.has(f.path)) {
        stalePaths.push(f.path);
      }
    }
    if (stalePaths.length === 0) return;
    // YYYY-MM is local-time so a user's "April archive" lands in the
    // folder they expect regardless of UTC offset. Pad to 2 digits.
    const now = new Date();
    const yyyy = now.getFullYear();
    const mm = String(now.getMonth() + 1).padStart(2, "0");
    const destDir = `${currentPath}/.archive/${yyyy}-${mm}`;

    try {
      // The existing `move_files` IPC requires the destination directory
      // to already exist (it explicitly errors out otherwise — that
      // strictness is correct for user-initiated cut/paste because a
      // typo'd path should be caught loudly). Smart Archive's intent is
      // different: "ensure this dated subfolder exists, I don't care
      // whether it did before". So we make a single idempotent
      // `ensure_directory` call first; it's a no-op when the dir
      // already exists.
      await tauriInvoke("ensure_directory", { path: destDir });
      const result = await assessBeforeExecute(
        {
          engine: "fs",
          kind: "move",
          affected_files: stalePaths.length,
          total_bytes: 0,
          subject_path: currentPath,
          summary: `Archive ${stalePaths.length} stale file${stalePaths.length === 1 ? "" : "s"} → .archive/${yyyy}-${mm}/`,
        },
        () => tauriInvoke("move_files", { sourcePaths: stalePaths, destDir }),
      );
      if (result === null) return; // user rejected the safety dialog
      pushUndo({
        id: `archive-stale-${Date.now()}`,
        type: "move",
        sourcePaths: stalePaths,
        destPaths: stalePaths.map((p) => `${destDir}/${p.split("/").pop()}`),
        timestamp: Date.now(),
      });
      // Drop the stale-only filter so the user immediately sees the
      // updated, post-archive view of the folder. Refreshing alone
      // would still show "0 stale" with the chip active, which is
      // technically correct but anti-climactic.
      setStaleOnly(false);
      // Iter 19: path-aware refresh. Passing both `currentPath`
      // (the source dir that lost stale files) and `destDir` (the
      // new .archive subfolder that gained them) so ANY pane
      // viewing either path auto-refreshes \u2014 including another
      // pane that happens to be open on the same directory.
      dispatchRefresh([currentPath, destDir]);
    } catch (err) {
      console.error("Smart Archive failed:", err);
    }
  }, [staleLoaded, currentPath, files, recentlyTouchedSet, pushUndo]);

  // Feature 1: Hidden file filtering (#85)
  const visibleFiles = useMemo(() => {
    if (showHiddenFiles) return staleFilteredFiles;
    return staleFilteredFiles.filter(f => !f.is_hidden && !f.name.startsWith('.'));
  }, [staleFilteredFiles, showHiddenFiles]);

  // Selection
  const { selectedPaths, handleSelect, selectAll, invertSelection } =
    useFileSelection(pane.id, visibleFiles);

  // Status-bar stats — publish folder + selection tallies into the store
  // every time the file list or selection changes. The store applies
  // equality-shortcircuit so the global status bar's subscriber stays
  // quiet between irrelevant renders. Uses `visibleFiles` (post-filter,
  // post-hidden) so the counts match what the user can actually see —
  // matching Finder's "5 of 27 items" convention when a filter is on.
  useEffect(() => {
    const folder = computeListBytes(visibleFiles);
    const sel = computeSelectionBytes(visibleFiles, selectedPaths);
    useFileManagerStore.getState().setPaneStats(pane.id, {
      totalCount: folder.count,
      totalBytes: folder.bytes,
      totalHasDir: folder.hasDir,
      selectedCount: sel.count,
      selectedBytes: sel.bytes,
      selectedHasDir: sel.hasDir,
    });
  }, [visibleFiles, selectedPaths, pane.id]);

  // Drag and drop
  const { handleDragOver, handleDrop } = useFileDragDrop(pane.id);

  // Navigation
  const handleNavigate = useCallback(
    (path: string) => {
      if (activeTab) {
        const label = path === "/" ? "Root" : path.split("/").filter(Boolean).pop() || "Root";
        navigateTab(paneIndex, activeTab.id, path, label);
        addRecentLocation(path);
      }
    },
    [paneIndex, activeTab, navigateTab, addRecentLocation],
  );

  // Open file/folder — iter 38: unified smart-open dispatcher.
  // Uses the module-level extension predicates (iter 37) as the
  // single source of truth so a future format addition only
  // changes one list. Precedence is deliberate:
  //   1. Directory   → navigate into it (existing)
  //   2. Archive     → in-app `ArchiveBrowser` (iter 31)
  //   3. Plain text  → in-app `TextEditor` if ≤1 MB (iter 32)
  //   4. Previewable → inline preview pane, BUT ONLY when the
  //                    pane is already open (iter 38 contextual
  //                    rule — users who never open preview pane
  //                    see zero behaviour change vs pre-iter-38)
  //   5. Anything else → OS default opener (existing fallback)
  // This contract keeps double-click predictable: the user's
  // existing OS-default behaviour for media files is preserved
  // until they explicitly opt in by opening the preview pane.
  const handleOpen = useCallback(
    (entry: FileEntryData) => {
      if (entry.is_dir) {
        handleNavigate(entry.path);
        return;
      }
      if (isArchivePath(entry.path)) {
        onArchiveBrowse(entry.path);
        return;
      }
      const TEXT_EDITOR_MAX_BYTES = 1024 * 1024;
      if (
        isPlainTextPath(entry.path) &&
        (entry.size ?? 0) <= TEXT_EDITOR_MAX_BYTES
      ) {
        onTextEdit(entry.path, entry.size ?? 0);
        return;
      }
      // Iter 38: contextual preview routing. Only takes the
      // click when the preview pane is ALREADY visible — this
      // treats "preview pane open" as an explicit "I want the
      // inline previewer, not Preview.app / Photos / Acrobat"
      // mode. Reads the pane state inline from the ui-store
      // rather than subscribing via a selector so handleOpen's
      // dep array stays small and stable across pane-open
      // toggles (the effect would invalidate every toggle).
      if (
        isPreviewablePath(entry.path) &&
        useUIStore.getState().previewPanelOpen
      ) {
        useUIStore.getState().openPreviewFor(entry.path);
        return;
      }
      if (isTauriAvailable()) {
        tauriInvoke("open_file_with_default", { path: entry.path }).catch((err) => {
          console.error("Failed to open file:", err);
        });
      }
    },
    [handleNavigate, onArchiveBrowse, onTextEdit],
  );

  // Feature 2: Calculate folder size (#83)
  const handleCalculateSize = useCallback(async (folderPath: string) => {
    const calculateRecursive = async (dirPath: string): Promise<number> => {
      try {
        let entries: FileEntryData[];
        if (isTauriAvailable()) {
          entries = await tauriInvoke<FileEntryData[]>("list_directory", { path: dirPath });
        } else {
          entries = generateDemoFiles(dirPath);
        }
        let totalSize = 0;
        for (const entry of entries) {
          if (entry.is_dir) {
            totalSize += await calculateRecursive(entry.path);
          } else {
            totalSize += entry.size || 0;
          }
        }
        return totalSize;
      } catch {
        return 0;
      }
    };

    const size = await calculateRecursive(folderPath);
    setFolderSizes((prev) => ({ ...prev, [folderPath]: size }));
  }, []);

  // Conflict check helper
  const checkConflicts = useCallback(async (sourcePaths: string[], destDir: string): Promise<boolean> => {
    if (!isTauriAvailable()) return true;
    for (const sourcePath of sourcePaths) {
      const fileName = sourcePath.split("/").pop() || "";
      const destPath = `${destDir}/${fileName}`;
      try {
        const conflict = await tauriInvoke<{
          has_conflict: boolean;
          source_exists: boolean;
          dest_exists: boolean;
          source_newer: boolean;
          size_differs: boolean;
        }>("check_conflict", { sourcePath, destPath });

        if (conflict.has_conflict) {
          const details: string[] = [];
          if (conflict.dest_exists) details.push("Destination file already exists");
          if (conflict.source_newer) details.push("Source is newer");
          if (conflict.size_differs) details.push("File sizes differ");

          const proceed = confirm(
            `Conflict detected for "${fileName}":\n${details.join("\n")}\n\nDo you want to overwrite?`
          );
          if (!proceed) return false;
        }
      } catch {
        // proceed on check failure
      }
    }
    return true;
  }, []);

  // File operations
  const handleCopy = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        const store = useFileManagerStore.getState();
        const otherPaneIndex = paneIndex === 0 ? 1 : 0;
        const otherPath = store.getActivePath(otherPaneIndex);
        const proceed = await checkConflicts(selectedPaths, otherPath);
        if (!proceed) return;
        await tauriInvoke("copy_files", { sourcePaths: selectedPaths, destDir: otherPath });
        pushUndo({ id: `copy-${Date.now()}`, type: "copy", sourcePaths: selectedPaths, destPaths: selectedPaths.map((p) => `${otherPath}/${p.split("/").pop()}`), timestamp: Date.now() });
        // Iter 18: path-aware refresh. `otherPath` covers the
        // destination pane; `currentPath` covers the source pane
        // (harmless for copy but needed for symmetry with move).
        // Unrelated panes skip the reload.
        dispatchRefresh([currentPath, otherPath]);
      } catch (err) {
        reportOperationFailure("Copy", err);
      }
    } else { console.log("Copy (demo mode):", selectedPaths); }
  }, [selectedPaths, paneIndex, pushUndo, checkConflicts, currentPath]);

  const handleCut = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        const store = useFileManagerStore.getState();
        const otherPaneIndex = paneIndex === 0 ? 1 : 0;
        const otherPath = store.getActivePath(otherPaneIndex);
        const proceed = await checkConflicts(selectedPaths, otherPath);
        if (!proceed) return;
        const result = await assessBeforeExecute(
          {
            engine: "fs",
            kind: "move",
            affected_files: selectedPaths.length,
            total_bytes: 0,
            subject_path: currentPath,
            summary: `Move ${selectedPaths.length} item${selectedPaths.length === 1 ? "" : "s"} → ${otherPath}`,
          },
          () => tauriInvoke("move_files", { sourcePaths: selectedPaths, destDir: otherPath }),
        );
        if (result === null) return;
        pushUndo({ id: `move-${Date.now()}`, type: "move", sourcePaths: selectedPaths, destPaths: selectedPaths.map((p) => `${otherPath}/${p.split("/").pop()}`), timestamp: Date.now() });
        // Iter 18: path-aware refresh covering BOTH the source
        // pane (files left) AND the destination pane (files
        // arrived). Fixes the pre-iter-18 bug where a direct
        // `loadDirectory(currentPath)` only refreshed the source
        // pane, leaving the destination pane showing stale state.
        dispatchRefresh([currentPath, otherPath]);
      } catch (err) {
        reportOperationFailure("Move", err);
      }
    } else { console.log("Cut (demo mode):", selectedPaths); }
  }, [selectedPaths, paneIndex, pushUndo, currentPath, checkConflicts]);

  const handleDelete = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        const result = await assessBeforeExecute(
          {
            engine: "fs",
            kind: "delete",
            affected_files: selectedPaths.length,
            total_bytes: 0,
            subject_path: currentPath,
            summary: `Move ${selectedPaths.length} item${selectedPaths.length === 1 ? "" : "s"} to Trash`,
          },
          () => tauriInvoke("delete_files", { paths: selectedPaths, permanent: false }),
        );
        if (result === null) return;
        pushUndo({ id: `delete-${Date.now()}`, type: "delete", sourcePaths: selectedPaths, destPaths: [], timestamp: Date.now() });
        // Iter 19: path-aware refresh. Another pane viewing the
        // same directory (e.g. after Open-in-Other-Pane from
        // iter 11) will notice the deletion automatically.
        dispatchRefresh([currentPath, ...selectedPaths]);
      } catch (err) {
        reportOperationFailure("Delete", err, {
          userAction:
            "Check the files aren't locked by another app and you have permission to remove them",
        });
      }
    } else { console.log("Delete (demo mode):", selectedPaths); }
  }, [selectedPaths, pushUndo, currentPath]);

  const handleDeletePermanently = useCallback(async (paths: string[]) => {
    if (paths.length === 0) return;
    const fileNames = paths.map((p) => p.split("/").pop()).join(", ");
    const confirmed = window.confirm(
      `Permanently delete ${paths.length} item(s)?\n\n${fileNames}\n\nThis action cannot be undone.`
    );
    if (!confirmed) return;
    if (isTauriAvailable()) {
      try {
        // Safety Interlock runs AFTER the inline confirm so the user has
        // two independent decision points for hard deletes: the OS-style
        // name list confirm, and the anomaly-aware interlock.
        const result = await assessBeforeExecute(
          {
            engine: "fs",
            kind: "purge",
            affected_files: paths.length,
            total_bytes: 0,
            subject_path: currentPath,
            summary: `Permanently delete ${paths.length} item${paths.length === 1 ? "" : "s"}`,
          },
          async () => {
            // Use cloud_delete_permanently for cloud provider files,
            // otherwise fall back to delete_files with permanent=true.
            try {
              await tauriInvoke("cloud_delete_permanently", { paths });
            } catch {
              await tauriInvoke("delete_files", { paths, permanent: true });
            }
            return true;
          },
        );
        if (result === null) return;
        // Iter 19: path-aware refresh so any other pane viewing
        // these paths (or their parent directory) also updates.
        dispatchRefresh([currentPath, ...paths]);
      } catch (err) {
        reportOperationFailure("Permanent delete", err, {
          appDid:
            "Did not change anything on disk; the files were NOT removed",
          userAction:
            "Check file permissions and that no app is holding the files open",
        });
      }
    } else { console.log("Delete permanently (demo mode):", paths); }
  }, [currentPath]);

  const handleRename = useCallback(async (filePath: string) => {
    if (isTauriAvailable()) {
      const oldName = filePath.split("/").pop() || "";
      const newName = window.prompt("Enter new name:", oldName);
      if (newName && newName !== oldName) {
        try {
          await tauriInvoke("rename_file", { sourcePath: filePath, newName });
          const newPath = filePath.replace(/[^/]+$/, newName);
          pushUndo({ id: `rename-${Date.now()}`, type: "rename", sourcePaths: [filePath], destPaths: [newPath], timestamp: Date.now() });
          // Iter 19: passing both old and new paths so any pane
          // viewing the parent directory refreshes (because either
          // path is a child of the parent, so the helper's prefix
          // match kicks in).
          dispatchRefresh([filePath, newPath]);
        } catch (err) {
          reportOperationFailure("Rename", err, {
            userAction:
              "Check the new name doesn't already exist and contains no path separators",
          });
        }
      }
    } else { console.log("Rename (demo mode):", filePath); }
  }, [pushUndo]);

  const handleDuplicate = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        await tauriInvoke("duplicate_files", { paths: selectedPaths });
        // Iter 19: duplicates land in the same directory as the
        // originals. Passing both selectedPaths and currentPath
        // covers every pane that might be viewing either.
        dispatchRefresh([currentPath, ...selectedPaths]);
      } catch (err) {
        reportOperationFailure("Duplicate", err);
      }
    } else { console.log("Duplicate (demo mode):", selectedPaths); }
  }, [selectedPaths, currentPath]);

  const handleNewFolder = useCallback(async () => {
    if (isTauriAvailable()) {
      const name = window.prompt("New folder name:", "New Folder");
      if (name) {
        try {
          const newPath = `${currentPath === "/" ? "" : currentPath}/${name}`;
          await tauriInvoke("create_directory", { path: newPath });
          pushUndo({ id: `mkdir-${Date.now()}`, type: "create_folder", sourcePaths: [], destPaths: [newPath], timestamp: Date.now() });
          // Iter 19: the new folder's parent (currentPath) is where
          // listings change; passing newPath as well for any pane
          // that might already be viewing it (unusual but safe).
          dispatchRefresh([currentPath, newPath]);
        } catch (err) {
          reportOperationFailure("Create folder", err, {
            userAction:
              "Check the name doesn't conflict with an existing item and you have write permission",
          });
        }
      }
    } else { console.log("New Folder (demo mode) in", currentPath); }
  }, [currentPath, pushUndo]);

  const handleNewFile = useCallback(async () => {
    if (isTauriAvailable()) {
      const name = window.prompt("New file name:", "untitled.txt");
      if (name) {
        try {
          const newPath = `${currentPath === "/" ? "" : currentPath}/${name}`;
          await tauriInvoke("create_file", { path: newPath });
          pushUndo({ id: `mkfile-${Date.now()}`, type: "create_file", sourcePaths: [], destPaths: [newPath], timestamp: Date.now() });
          // Iter 19: same pattern as handleNewFolder.
          dispatchRefresh([currentPath, newPath]);
        } catch (err) {
          reportOperationFailure("Create file", err, {
            userAction:
              "Check the name doesn't conflict with an existing item and you have write permission",
          });
        }
      }
    } else { console.log("New File (demo mode) in", currentPath); }
  }, [currentPath, pushUndo]);

  const handleGetInfo = useCallback(async (filePath: string) => {
    const fileName = filePath.split("/").pop() || filePath;
    let metadata: Record<string, any> | null = null;
    let comment = "";

    if (isTauriAvailable()) {
      try {
        metadata = await tauriInvoke<Record<string, unknown>>("get_file_metadata", { path: filePath });
      } catch (err) {
        console.error("Get info failed:", err);
      }
      try {
        const fileInfo = await tauriInvoke<{ label?: string }>("integrity_get_file_info", { path: filePath });
        comment = fileInfo?.label || "";
      } catch { /* no comment yet */ }
    } else {
      metadata = { size: "4.2 KB", modified: new Date().toISOString(), type: "File" };
    }

    setInfoDialog({ path: filePath, name: fileName, metadata, comment });
  }, []);

  // Feature 5: Save comment (#82)
  const handleSaveComment = useCallback(async (filePath: string, commentText: string) => {
    if (isTauriAvailable()) {
      try {
        await tauriInvoke("integrity_set_label", { path: filePath, label: commentText });
      } catch {
        try {
          await tauriInvoke("set_config", { key: `comment:${filePath}`, value: commentText });
        } catch (err) {
          console.error("Save comment failed:", err);
        }
      }
    } else {
      console.log("Save comment (demo mode):", filePath, commentText);
    }
  }, []);

  // Set permissions (chmod) handler
  const handleSetPermissions = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    const mode = window.prompt("Enter permissions (octal):", "755");
    if (!mode?.trim()) return;
    if (isTauriAvailable()) {
      try {
        for (const filePath of selectedPaths) {
          await tauriInvoke("set_permissions", {
            protocol: "sftp",
            path: filePath,
            mode: mode.trim(),
            recursive: false,
          });
        }
        // Iter 19: permission changes are metadata-only but may
        // affect the displayed permission column. Any pane viewing
        // the parent directory should re-fetch so the new mode
        // appears.
        dispatchRefresh([currentPath, ...selectedPaths]);
      } catch (err) {
        console.error("Set permissions failed:", err);
      }
    } else {
      console.log("Set permissions (demo mode):", selectedPaths, mode);
    }
  }, [selectedPaths, currentPath]);

  // Create symlink handler
  const handleCreateSymlink = useCallback(async () => {
    if (selectedPaths.length !== 1) return;
    const target = selectedPaths[0];
    const linkName = window.prompt(
      "Enter path for the symbolic link:",
      target + ".link"
    );
    if (!linkName?.trim()) return;
    if (isTauriAvailable()) {
      try {
        const linkPath = linkName.trim();
        await tauriInvoke("create_symlink", {
          target,
          linkPath,
        });
        // Iter 19: the symlink may land in a DIFFERENT directory
        // than the target's parent (user can type any path at the
        // prompt). Passing both the link path AND currentPath so
        // any pane viewing either refreshes.
        dispatchRefresh([currentPath, linkPath]);
      } catch (err) {
        console.error("Create symlink failed:", err);
      }
    } else {
      console.log("Create symlink (demo mode):", target, "->", linkName);
    }
  }, [selectedPaths, currentPath]);

  // Copy as script handler
  const handleCopyAsScript = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    let script: string;
    if (isTauriAvailable()) {
      try {
        script = await tauriInvoke<string>("generate_script", {
          operation: "copy",
          sourcePaths: selectedPaths,
          destPath: null,
          options: null,
        });
      } catch (err) {
        useUIStore.getState().addStructuredError({
          what: "Copy as Script failed",
          why: err instanceof Error ? err.message : "Script generation failed",
          appDid: "Did not write anything to the clipboard",
          userAction: "Confirm the selection is non-empty and try again",
        });
        return;
      }
    } else {
      script = selectedPaths.map((p) => `cp -r "${p}" .`).join("\n");
    }
    await copyToClipboardWithToast(script, "Copy as Script");
  }, [selectedPaths]);

  // Iter 15: "Extract Here" right-click handler. Mirror of iter 14's
  // Compress wiring — uses the same `archive_extract` IPC the
  // ArchiveBrowser modal calls, with `dest_dir` computed to a same-
  // name subfolder so contents don't clobber files in the parent.
  // Routes failures through `reportOperationFailure` for the standard
  // structured toast; success fires a path-aware refresh so any pane
  // viewing the parent directory shows the new folder immediately.
  const handleExtractHere = useCallback(
    async (archivePath: string) => {
      const destDir = archiveExtractDest(archivePath);
      if (!isTauriAvailable()) return;
      try {
        const result = await tauriInvoke<{
          success: boolean;
          files_processed: number;
          errors: string[];
        }>("archive_extract", {
          params: {
            archive_path: archivePath,
            dest_dir: destDir,
            password: null,
            selected_entries: null,
          },
        });
        if (!result.success) {
          useUIStore.getState().addStructuredError({
            what: "Extract failed",
            why: result.errors.join("\n") || "The backend reported no progress",
            appDid: "Did not create any files",
            userAction:
              "If the archive is password-protected, double-click it to open the browser and supply a password",
          });
          return;
        }
        const parent = destDir.substring(0, destDir.lastIndexOf("/")) || "/";
        dispatchRefresh([parent]);
        useUIStore.getState().addStructuredError({
          what: `Extracted ${result.files_processed} item${
            result.files_processed === 1 ? "" : "s"
          }`,
          why: `Archive contents placed in ${destDir}`,
          appDid: "Refreshed the parent directory so the new folder appears",
          userAction: "Open the new folder to inspect the extracted contents",
        });
      } catch (err) {
        reportOperationFailure("Extract Here", err, {
          userAction:
            "Verify the archive isn't corrupted; password-protected archives need the ArchiveBrowser modal",
        });
      }
    },
    [],
  );

  // Edit remote (#44)
  const handleEditRemote = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    const filePath = selectedPaths[0];
    if (isTauriAvailable()) {
      try {
        await tauriInvoke("open_file_with_default", { path: filePath });
        console.log("Editing remotely:", filePath);
      } catch (err) { console.error("Edit remote failed:", err); }
    } else { console.log("Edit Remote (demo mode):", filePath); }
  }, [selectedPaths]);

  // Copy Path handlers — every one of these previously swallowed
  // both success and failure silently (try/catch with an empty
  // `catch {}`). They now route through `copyToClipboardWithToast`
  // so the user gets visible confirmation on success and an
  // actionable error toast on failure. The transform-then-copy
  // handlers (remote / url / relative) still surface a structured
  // error if their backend IPC fails before the clipboard write —
  // they explicitly distinguish "IPC failed" from "clipboard failed".
  const handleCopyPath = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    await copyToClipboardWithToast(selectedPaths[0], "Copy Path");
  }, [selectedPaths]);

  const handleCopyRemotePath = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    let resolved: string;
    try {
      const result = await tauriInvoke<{ path: string }>("copy_remote_path", {
        path: selectedPaths[0],
        remoteBase: null,
      });
      resolved = result.path;
    } catch (err) {
      useUIStore.getState().addStructuredError({
        what: "Copy Remote Path failed",
        why: err instanceof Error ? err.message : "Backend rejected the request",
        appDid: "Did not write anything to the clipboard",
        userAction: "Confirm a remote base is configured for this connection",
      });
      return;
    }
    await copyToClipboardWithToast(resolved, "Copy Remote Path");
  }, [selectedPaths]);

  const handleCopyUrl = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    let resolved: string;
    try {
      const result = await tauriInvoke<{ path: string }>("copy_url", {
        path: selectedPaths[0],
        protocol: "local",
        host: "localhost",
        port: null,
        username: null,
      });
      resolved = result.path;
    } catch (err) {
      useUIStore.getState().addStructuredError({
        what: "Copy URL failed",
        why: err instanceof Error ? err.message : "Backend rejected the request",
        appDid: "Did not write anything to the clipboard",
        userAction: "Try the simple Copy Path action instead",
      });
      return;
    }
    await copyToClipboardWithToast(resolved, "Copy URL");
  }, [selectedPaths]);

  const handleCopyRelativePath = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    let resolved: string;
    try {
      const result = await tauriInvoke<{ path: string }>("copy_relative_path", {
        path: selectedPaths[0],
        basePath: currentPath,
      });
      resolved = result.path;
    } catch (err) {
      useUIStore.getState().addStructuredError({
        what: "Copy Relative Path failed",
        why: err instanceof Error ? err.message : "Backend rejected the request",
        appDid: "Did not write anything to the clipboard",
        userAction: "Confirm the current pane has a valid base path",
      });
      return;
    }
    await copyToClipboardWithToast(resolved, "Copy Relative Path");
  }, [selectedPaths, currentPath]);

  // Open With / Open in Editor handlers
  const handleOpenWith = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    try {
      await tauriInvoke("open_in_editor", {
        filePath: selectedPaths[0],
        editorPath: null,
        editorArgs: null,
      });
    } catch {}
  }, [selectedPaths]);

  const handleOpenInEditor = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    const ext = selectedPaths[0].split('.').pop() || "";
    try {
      const mapping = await tauriInvoke<{ app_path: string; args: string[] } | null>("resolve_editor", { extension: ext });
      await tauriInvoke("open_in_editor", {
        filePath: selectedPaths[0],
        editorPath: mapping?.app_path || null,
        editorArgs: mapping?.args || null,
      });
    } catch {}
  }, [selectedPaths]);

  // Compare files (#47)
  const handleCompareFiles = useCallback(async () => {
    if (selectedPaths.length !== 2) return;
    const [pathA, pathB] = selectedPaths;
    const nameA = pathA.split("/").pop() || pathA;
    const nameB = pathB.split("/").pop() || pathB;
    if (isTauriAvailable()) {
      try {
        const [contentA, contentB] = await Promise.all([
          tauriInvoke<string>("preview_file", { path: pathA }),
          tauriInvoke<string>("preview_file", { path: pathB }),
        ]);
        setCompareData({ left: contentA, right: contentB, leftName: nameA, rightName: nameB });
      } catch (err) { console.error("Compare files failed:", err); }
    } else {
      setCompareData({
        left: `// Contents of ${nameA}\nconst hello = "world";\nconsole.log(hello);`,
        right: `// Contents of ${nameB}\nconst hello = "universe";\nconsole.log(hello);`,
        leftName: nameA, rightName: nameB,
      });
    }
  }, [selectedPaths]);

  // Feature 4: Content search handler (#69)
  const handleContentSearch = useCallback((query: string, path: string) => {
    console.log(`Content search: "${query}" in ${path}`);
    // Content search requires backend support - log for now
  }, []);

  // Smart Send-To handler — moves the current selection to a
  // ledger-suggested destination directory in one click. Reuses the
  // exact same safety + undo plumbing as the dual-pane Cut→Paste path
  // (assessBeforeExecute → move_files → pushUndo → loadDirectory) so
  // the surface stays DRY and behavior is identical to existing moves.
  const handleSendTo = useCallback(
    async (destDir: string) => {
      if (selectedPaths.length === 0) return;
      if (!isTauriAvailable()) {
        console.log("Send-To (demo mode):", selectedPaths, "→", destDir);
        return;
      }
      try {
        const proceed = await checkConflicts(selectedPaths, destDir);
        if (!proceed) return;
        const result = await assessBeforeExecute(
          {
            engine: "fs",
            kind: "move",
            affected_files: selectedPaths.length,
            total_bytes: 0,
            subject_path: currentPath,
            summary: `Send ${selectedPaths.length} item${selectedPaths.length === 1 ? "" : "s"} → ${destDir}`,
          },
          () => tauriInvoke("move_files", { sourcePaths: selectedPaths, destDir }),
        );
        if (result === null) return;
        pushUndo({
          id: `send-to-${Date.now()}`,
          type: "move",
          sourcePaths: selectedPaths,
          destPaths: selectedPaths.map((p) => `${destDir}/${p.split("/").pop()}`),
          timestamp: Date.now(),
        });
        // Iter 19: Smart Send-To is a cross-directory bulk move
        // \u2014 the iter-18-class bug. Files leave `currentPath` and
        // arrive at `destDir`. Passing BOTH so whichever pane is
        // viewing the destination (if any) refreshes alongside
        // the source pane.
        dispatchRefresh([currentPath, destDir]);
      } catch (err) {
        console.error("Smart Send-To failed:", err);
      }
    },
    [selectedPaths, currentPath, pushUndo, checkConflicts],
  );

  // Context menu
  const handleContextMenu = useCallback(
    async (entry: FileEntryData, event: React.MouseEvent) => {
      // Smart Send-To: kick off the destination lookup BEFORE building
      // the menu so the inline entries appear on first right-click.
      // First call for an extension does a single ledger query (~5–30ms);
      // subsequent right-clicks on the same extension are instant via
      // the module-level cache. The menu opens after the await so the
      // user never sees a flicker as items get inserted.
      let smartDestinations: SmartDestination[] = [];
      if (!entry.is_dir) {
        const ext = fileExtension(entry.path);
        if (ext !== "") {
          smartDestinations = await loadSmartDestinations(ext, 3);
          // Filter out the current directory — never propose moving a
          // file to where it already lives.
          smartDestinations = smartDestinations.filter(
            (d) => d.path !== currentPath,
          );
        }
      }
      const items = getFileContextMenuItems({
        hasSelection: true,
        isDirectory: entry.is_dir,
        selectionCount: selectedPaths.length || 1,
        onCopy: handleCopy,
        onCut: handleCut,
        onPaste: () => { console.log("Paste into", currentPath); },
        onDelete: handleDelete,
        onDeletePermanently: () => handleDeletePermanently(selectedPaths.length > 0 ? selectedPaths : [entry.path]),
        onRename: () => handleRename(entry.path),
        onDuplicate: handleDuplicate,
        onNewFolder: handleNewFolder,
        onNewFile: handleNewFile,
        onAddToFavorites: () => {
          useFileManagerStore.getState().addFavorite({ id: `fav-${Date.now()}`, name: entry.name, path: entry.path });
        },
        onGetInfo: () => handleGetInfo(entry.path),
        onOpen: () => handleOpen(entry),
        onRefresh: () => loadDirectory(currentPath),
        onSelectAll: selectAll,
        onInvertSelection: invertSelection,
        // Feature 2: Calculate folder size (#83)
        onCalculateSize: entry.is_dir ? () => handleCalculateSize(entry.path) : undefined,
        onEditRemote: handleEditRemote,
        onCompareFiles: handleCompareFiles,
        // Feature 2: Git actions (#46)
        onGitAdd: isGitRepo ? () => {
          console.log("Git stage:", entry.path);
        } : undefined,
        onGitCommit: isGitRepo ? () => {
          console.log("Git commit from:", currentPath);
        } : undefined,
        onExplainError: () => {
          const aiStore = useAiStore.getState();
          aiStore.explainError("file_operation", `Operation on ${entry.name}`, "Check file permissions and try again");
          aiStore.setPanelOpen(true);
        },
        onOpenInTerminal: entry.is_dir
          ? () => {
              const termStore = useTerminalStore.getState();
              termStore.createLocalSession(entry.path);
              termStore.setPanelOpen(true);
            }
          : undefined,
        // WinSCP parity: permissions, symlinks, script generation
        onSetPermissions: handleSetPermissions,
        onCreateSymlink: handleCreateSymlink,
        onCopyAsScript: handleCopyAsScript,
        // Copy path variants
        onCopyPath: handleCopyPath,
        onCopyRemotePath: handleCopyRemotePath,
        onCopyUrl: handleCopyUrl,
        onCopyRelativePath: handleCopyRelativePath,
        // Open with / editor
        onOpenWith: handleOpenWith,
        onOpenInEditor: handleOpenInEditor,
        // Iter 39: Verify integrity — snapshots the right-clicked
        // selection OR the single entry the user clicked on when
        // nothing is selected, then opens the IntegrityTools modal
        // with the Checksums tab pre-populated. Falls back to a
        // single-file array when `selectedPaths` is empty because
        // right-clicking an unselected entry is a common "do this
        // one thing" gesture.
        onVerifyIntegrity: () => {
          const files = selectedPaths.length > 0 ? selectedPaths : [entry.path];
          onVerifyIntegrity(files, currentPath);
        },
        // Iter 41: Compress to archive. Same selection-snapshot rule
        // as Verify Integrity above — if the user has a selection the
        // dialog uses it; otherwise it falls back to the single right-
        // clicked entry.
        onCompress: () => {
          const files = selectedPaths.length > 0 ? selectedPaths : [entry.path];
          onCompress(files);
        },
        // Iter 15: Extract Here. Only wired when the right-clicked
        // entry is an archive file — the gating happens here so the
        // context-menu doesn't need to inspect file types itself,
        // matching the iter-39 onCompareFiles / onEditRemote
        // pattern. Direct action — no dialog; for password-
        // protected archives the user double-clicks instead.
        onExtract:
          !entry.is_dir && isArchivePath(entry.path)
            ? () => handleExtractHere(entry.path)
            : undefined,
        // Show File History — opens the lineage panel for this exact path.
        // The panel fetches once via `get_file_lineage`, then self-hides on
        // Escape or its own close button. No state persists between calls.
        onShowHistory: () => {
          useLineageStore.getState().beginRequest(entry.path);
        },
        // Show Folder Activity — opens the Activity Timeline pre-filtered
        // to events under this folder. The panel's own filter chip is
        // dismissible, and `openWithPreset` also clears any prior
        // engine / failed / correlation filter so the user starts from
        // the same "show me everything in this folder" baseline every
        // time, not whatever filter happened to be active before.
        onShowFolderActivity: entry.is_dir
          ? () => {
              useActivityTimelineStore.getState().openWithPreset({
                pathFilter: entry.path,
                engineFilter: "all",
                failedOnly: false,
                correlationFilter: null,
              });
            }
          : undefined,
        // Reveal in the native OS file manager. Backend chooses the
        // platform-specific subprocess (`open -R` / `explorer /select`
        // / `xdg-open` on the parent dir). Routes failures through
        // the standard structured-toast surface used by the rest of
        // the destructive-op handlers — invisible silence here would
        // be hostile (the user clicks the action and "nothing
        // happens" is the worst possible feedback).
        onRevealInOs: async () => {
          if (!isTauriAvailable()) return;
          try {
            await tauriInvoke("reveal_in_os", { path: entry.path });
          } catch (err) {
            reportOperationFailure("Show in Finder/Explorer", err, {
              userAction:
                "Check the file still exists and your OS file manager is on PATH",
            });
          }
        },
        // Smart Send-To: ledger-derived frecency-ranked destinations.
        smartDestinations,
        onSendTo: handleSendTo,
      });
      onContextMenu({ position: { x: event.clientX, y: event.clientY }, items });
    },
    [selectedPaths, currentPath, handleOpen, handleCopy, handleCut, handleDelete, handleDeletePermanently, handleRename, handleDuplicate, handleNewFolder, handleNewFile, handleGetInfo, handleCalculateSize, handleEditRemote, handleCompareFiles, handleSetPermissions, handleCreateSymlink, handleCopyAsScript, handleCopyPath, handleCopyRemotePath, handleCopyUrl, handleCopyRelativePath, handleOpenWith, handleOpenInEditor, selectAll, invertSelection, onContextMenu, loadDirectory, isGitRepo, handleSendTo, onVerifyIntegrity, onCompress, handleExtractHere],
  );

  // Keyboard shortcuts for compare (#47) and quick select (#71)
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "=") {
        e.preventDefault();
        if (selectedPaths.length === 2) { handleCompareFiles(); }
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "S") {
        e.preventDefault();
        setQuickSelectOpen(prev => !prev);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectedPaths, handleCompareFiles]);

  return (
    <div
      className="flex flex-col h-full relative"
      onDragOver={handleDragOver}
      onDrop={(e) => {
        const droppedPaths = handleDrop(e);
        if (droppedPaths) {
          if (isTauriAvailable()) {
            const isCopy = e.altKey;
            const command = isCopy ? "copy_files" : "move_files";
            tauriInvoke(command, { sourcePaths: droppedPaths, destDir: currentPath })
              .then(() => {
                // Iter 19: drag-drop between panes needs to
                // refresh BOTH the source (move only) and the
                // destination. Passing droppedPaths covers the
                // source side because the source paths are
                // children of whatever directory they came from,
                // and the path-aware helper's prefix match finds
                // those panes. currentPath is the destination.
                dispatchRefresh([currentPath, ...droppedPaths]);
              })
              .catch((err) => { console.error(`${isCopy ? "Copy" : "Move"} via DnD failed:`, err); });
          } else {
            console.log("Dropped files (demo mode):", droppedPaths, "into", currentPath);
          }
        }
      }}
    >
      <TabBar paneIndex={paneIndex} />
      <BreadcrumbBar
        path={currentPath}
        onNavigate={handleNavigate}
        onBack={() => useFileManagerStore.getState().navigateBack(paneIndex)}
        onForward={() => useFileManagerStore.getState().navigateForward(paneIndex)}
        canGoBack={useFileManagerStore.getState().canNavigateBack(paneIndex)}
        canGoForward={useFileManagerStore.getState().canNavigateForward(paneIndex)}
      />
      <FilterBar
        paneIndex={paneIndex}
        totalCount={files.length}
        filteredCount={visibleFiles.length}
        onContentSearch={handleContentSearch}
        staleCount={staleCount}
        staleActive={staleOnly}
        onToggleStale={() => setStaleOnly((v) => !v)}
        onArchiveStale={handleArchiveStale}
      />

      {/* Feature 3: Grouping dropdown (#77) - advanced mode only */}
      {appModePane === "advanced" && (
        <div className="flex items-center gap-2 px-3 py-1 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
          <label className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]">Group:</label>
          <select
            value={pane.groupBy || ""}
            onChange={(e) => setGroupBy(paneIndex, e.target.value || null)}
            className="text-xs h-6 px-1 rounded border border-[var(--color-border)] bg-[var(--color-bg)] text-[color:var(--color-text)]"
          >
            <option value="">No grouping</option>
            <option value="type">By Type</option>
            <option value="extension">By Extension</option>
            <option value="date">By Date</option>
            <option value="size">By Size</option>
          </select>
        </div>
      )}

      {/* Feature 3: Quick Select popover (#71) */}
      {quickSelectOpen && (
        <div className="absolute top-0 right-0 z-40 m-2 flex items-center gap-2 px-3 py-1.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-elevated)] shadow-lg">
          <span className="text-xs text-[color:var(--color-text-secondary)]">Select:</span>
          <input
            autoFocus
            value={quickSelectPattern}
            onChange={(e) => {
              setQuickSelectPattern(e.target.value);
              const pattern = e.target.value.toLowerCase();
              if (pattern) {
                const matching = visibleFiles
                  .filter(f => f.name.toLowerCase().includes(pattern) || (f.extension && f.extension.toLowerCase() === pattern))
                  .map(f => f.path);
                useFileManagerStore.getState().setSelection(pane.id, matching);
              }
            }}
            onKeyDown={(e) => {
              if (e.key === 'Escape') { setQuickSelectOpen(false); setQuickSelectPattern(""); }
              if (e.key === 'Enter') { setQuickSelectOpen(false); }
            }}
            placeholder="*.tsx, readme, etc."
            className="h-6 w-40 px-2 text-xs bg-transparent border border-[var(--color-border)] rounded outline-none"
          />
          <button onClick={() => { setQuickSelectOpen(false); setQuickSelectPattern(""); }} className="text-xs text-[color:var(--color-text-tertiary)]">x</button>
        </div>
      )}

      {loading && (
        <div className="h-0.5 bg-[var(--color-primary)] animate-pulse" />
      )}

      {error && !isTauriAvailable() && (
        <div className="px-3 py-1.5 text-[length:var(--font-size-xs)] text-[color:var(--color-error)] bg-[var(--color-error)]/10 border-b border-[var(--color-border)]">
          {error}
        </div>
      )}

      {/* Path Recall — ledger-powered "where did my file go?" banner.
          Rendered only when (a) the most recent loadDirectory call
          failed and (b) the unified ledger has a usable history for
          the path. Closes the cognitive gap left by the demo-files
          fallback by telling the user precisely why the pane is
          empty, what happened to the original file, and (when the
          ledger knows) where it lives now. */}
      {pathRecall && (
        <div
          className="flex items-start gap-2 px-3 py-2 text-[length:var(--font-size-xs)] bg-amber-500/10 border-b border-amber-500/20"
          role="status"
          aria-live="polite"
          data-testid="path-recall-banner"
        >
          <div className="mt-0.5 text-amber-500" aria-hidden="true">📍</div>
          <div className="flex-1 min-w-0">
            <div className="font-medium text-[color:var(--color-text)]">
              This path {describePathRecall(pathRecall)}
            </div>
            <div className="text-[10px] text-[color:var(--color-text-muted)]">
              Last seen {formatRelativeTime(pathRecall.lastSeenIso)}
            </div>
          </div>
          <div className="flex flex-shrink-0 items-center gap-1">
            {pathRecall.alternateLocation && (
              <button
                type="button"
                onClick={() => handleNavigate(pathRecall.alternateLocation!)}
                className="rounded px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300 hover:bg-amber-500/20 focus:outline-none focus:ring-1 focus:ring-amber-500/40"
                data-testid="path-recall-jump"
              >
                Jump there
              </button>
            )}
            <button
              type="button"
              onClick={() => useLineageStore.getState().beginRequest(currentPath)}
              className="rounded px-2 py-0.5 text-[11px] text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg,rgba(255,255,255,0.06))] focus:outline-none focus:ring-1 focus:ring-sky-500/40"
              data-testid="path-recall-lineage"
            >
              Show lineage
            </button>
            <button
              type="button"
              onClick={() => setPathRecall(null)}
              aria-label="Dismiss path recall"
              className="rounded p-0.5 text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
            >
              ×
            </button>
          </div>
        </div>
      )}

      <FileList
        files={visibleFiles}
        viewMode={pane.viewMode}
        selectedPaths={selectedPaths}
        onSelect={handleSelect}
        onOpen={handleOpen}
        onContextMenu={handleContextMenu}
        focusedIndex={focusedIndex}
        onFocusedIndexChange={setFocusedIndex}
        gitStatus={gitStatus}
        activityMap={activityMap}
        groupBy={pane.groupBy}
        folderSizes={folderSizes}
        onActivityDotClick={(path) =>
          useLineageStore.getState().beginRequest(path)
        }
      />

      {/* Feature 5: Get Info dialog with editable comments (#82) */}
      {infoDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-[var(--color-bg)] rounded-lg shadow-xl w-[400px] max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between px-4 py-2 border-b border-[var(--color-border)]">
              <h3 className="text-sm font-semibold text-[color:var(--color-text)]">Info: {infoDialog.name}</h3>
              <button
                onClick={() => setInfoDialog(null)}
                className="text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] px-2 py-0.5 rounded hover:bg-[var(--color-hover-bg)]"
                aria-label="Close info"
              >
                Close
              </button>
            </div>
            <div className="px-4 py-3 overflow-y-auto space-y-2">
              <div className="text-xs text-[color:var(--color-text-secondary)]">
                <strong>Path:</strong> {infoDialog.path}
              </div>
              {infoDialog.metadata && Object.entries(infoDialog.metadata).map(([key, val]) => (
                <div key={key} className="text-xs text-[color:var(--color-text-secondary)]">
                  <strong>{key}:</strong> {String(val)}
                </div>
              ))}
              <div className="mt-2">
                <label className="text-xs font-medium text-[color:var(--color-text)]">Comments</label>
                <textarea
                  value={infoDialog.comment}
                  onChange={(e) => setInfoDialog((prev) => prev ? { ...prev, comment: e.target.value } : null)}
                  onBlur={() => {
                    if (infoDialog) {
                      handleSaveComment(infoDialog.path, infoDialog.comment);
                    }
                  }}
                  className="w-full h-20 mt-1 p-2 text-xs border border-[var(--color-border)] rounded resize-none bg-[var(--color-bg)] text-[color:var(--color-text)]"
                  placeholder="Add a comment..."
                />
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Compare Files modal — finally renders the previously
          orphaned `_compareData` state. The pane owns its own
          compare modal so each pane's "Compare Selected" action
          opens in-place; the modal renders nothing when data is
          null, so the second pane's instance has zero runtime cost
          unless its own user invokes Compare. */}
      <CompareFilesModal data={compareData} onClose={() => setCompareData(null)} />
    </div>
  );
}

// ──────────────────────────────────────────────
// View Mode Selector
// ──────────────────────────────────────────────

function ViewModeSelector() {
  const activePaneIndex = useFileManagerStore((s) => s.activePaneIndex);
  const viewMode = useFileManagerStore((s) => s.panes[s.activePaneIndex].viewMode);
  const setViewMode = useFileManagerStore((s) => s.setViewMode);

  const modes: { mode: ViewMode; icon: React.ReactNode; label: string }[] = [
    { mode: "list", icon: <List className="h-3.5 w-3.5" />, label: "List view" },
    { mode: "detail", icon: <AlignJustify className="h-3.5 w-3.5" />, label: "Detail view" },
    { mode: "grid", icon: <Grid3X3 className="h-3.5 w-3.5" />, label: "Grid view" },
    { mode: "compact", icon: <Layout className="h-3.5 w-3.5" />, label: "Compact view" },
  ];

  return (
    <div className="flex items-center border border-[var(--color-border)] rounded-[var(--radius-md)] overflow-hidden">
      {modes.map((m) => (
        <button
          key={m.mode}
          className={cn(
            "h-7 w-7 flex items-center justify-center transition-theme",
            viewMode === m.mode
              ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]"
              : "text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={() => setViewMode(activePaneIndex, m.mode)}
          aria-label={m.label}
          aria-pressed={viewMode === m.mode}
          title={m.label}
        >
          {m.icon}
        </button>
      ))}
    </div>
  );
}

// ──────────────────────────────────────────────
// Status bar
// ──────────────────────────────────────────────

function StatusBarContent() {
  const paneStats = useFileManagerStore(
    (s) => s.paneStats[s.panes[s.activePaneIndex].id] ?? null,
  );

  // Iter 16: active pane's current path drives the free-space lookup.
  // Reading via the standard `panes[i].tabs.find(t => t.id === ...)`
  // pattern stays consistent with how FilePane resolves its own
  // `currentPath` — same source of truth, no risk of divergence.
  const activePath = useFileManagerStore((s) => {
    const pane = s.panes[s.activePaneIndex];
    return pane.tabs.find((t) => t.id === pane.activeTabId)?.path ?? "";
  });
  const drive = useDriveSpace(activePath);
  const driveLabel = drive ? formatDriveSpace(drive) : null;
  const driveDetail = drive ? formatDriveSpaceDetail(drive) : null;

  const [appVersion, setAppVersion] = useState<string>("UFOP v0.1.0");
  const [platformInfo, setPlatformInfo] = useState<{ os: string; arch: string; version: string } | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [configEntries, setConfigEntries] = useState<Record<string, string>>({});
  const [configKey, setConfigKey] = useState("");
  const [configValue, setConfigValue] = useState("");
  const [configError, setConfigError] = useState<string | null>(null);

  useEffect(() => {
    if (isTauriAvailable()) {
      tauriInvoke<string>("get_app_version").then(
        (v) => setAppVersion(`UFOP v${v}`),
        () => {}
      );
      tauriInvoke<{ os: string; arch: string; version: string }>("get_platform_info").then(
        (info) => setPlatformInfo(info),
        () => {}
      );
    }
  }, []);

  useEffect(() => {
    if (isTauriAvailable()) {
      tauriInvoke<Record<string, any>>("load_workspace_state").then(
        (state) => { if (state) { console.log("Workspace state loaded:", state); } },
        () => {}
      );
    }
  }, []);

  useEffect(() => {
    const handleBlur = () => {
      if (isTauriAvailable()) {
        const fmState = useFileManagerStore.getState();
        const workspaceData = {
          activePaneIndex: fmState.activePaneIndex,
          panes: fmState.panes.map((p) => ({
            id: p.id, activeTabId: p.activeTabId, viewMode: p.viewMode,
            tabs: p.tabs.map((t) => ({ id: t.id, path: t.path, label: t.label })),
          })),
        };
        tauriInvoke("save_workspace_state", { state: JSON.stringify(workspaceData) }).catch(() => {});
      }
    };
    window.addEventListener("blur", handleBlur);
    return () => window.removeEventListener("blur", handleBlur);
  }, []);

  const loadConfig = async () => {
    if (!isTauriAvailable()) return;
    try {
      const cfg = await tauriInvoke<Record<string, string>>("get_config", undefined, {});
      setConfigEntries(cfg);
    } catch (e: any) { setConfigError(e?.message || "Failed to load config"); }
  };

  const handleSetConfig = async () => {
    if (!configKey.trim()) return;
    setConfigError(null);
    try {
      await tauriInvoke("set_config", { key: configKey.trim(), value: configValue });
      setConfigKey(""); setConfigValue(""); await loadConfig();
    } catch (e: any) { setConfigError(e?.message || "Failed to save config"); }
  };

  const toggleSettings = async () => {
    const willOpen = !showSettings;
    setShowSettings(willOpen);
    if (willOpen) { await loadConfig(); }
  };

  const platformTooltip = platformInfo
    ? `${platformInfo.os} ${platformInfo.version} (${platformInfo.arch})`
    : undefined;

  return (
    <>
      {/* Left cluster: iter-13 summary + iter-16 free-space share a
          single flex group so the outer `justify-between` keeps three
          siblings (left cluster, pulse, right cluster) regardless of
          whether the free-space indicator is rendered. Without this
          wrapper, adding a 4th top-level child would respace the bar. */}
      <div className="flex items-center gap-3">
        <span
          className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]"
          data-testid="status-bar-summary"
          aria-live="polite"
        >
          {formatPaneStatsLabel(paneStats, formatBytes)}
        </span>

        {/* Iter 16: free-space indicator. Renders only when the
            active pane is on a recognised mount AND that mount
            reports accounting (`total_bytes > 0`). The terse "X GB
            free" sits next to the iter-13 summary; the rich
            "{drive name}: X free of Y" lives in the tooltip so
            casual users see the glanceable number while power
            users get the full picture on hover. */}
        {driveLabel && (
          <span
            className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] cursor-help"
            data-testid="status-bar-free-space"
            title={driveDetail ?? undefined}
            aria-label={driveDetail ?? driveLabel}
          >
            {driveLabel}
          </span>
        )}
      </div>

      {/* Engine Pulse — ambient cross-engine activity heartbeat */}
      <EnginePulseIndicator />

      <div className="flex items-center gap-2">
        <button
          onClick={toggleSettings}
          className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text-secondary)] p-0.5 rounded hover:bg-[var(--color-hover-bg)]"
          aria-label="Settings"
          title="Application Settings"
        >
          <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="3" />
            <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
          </svg>
        </button>
        <span
          className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]"
          title={platformTooltip}
        >
          {isTauriAvailable() ? appVersion : `${appVersion} (dev mode)`}
        </span>
      </div>

      {showSettings && (
        <div className="absolute bottom-[var(--status-bar-height)] right-2 w-80 bg-[var(--color-bg)] border border-[var(--color-border)] rounded-lg shadow-lg z-50">
          <div className="flex items-center justify-between px-3 py-2 border-b border-[var(--color-border)]">
            <span className="text-xs font-semibold">Settings</span>
            <button onClick={() => setShowSettings(false)} className="text-xs text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]">Close</button>
          </div>
          {platformInfo && (
            <div className="px-3 py-2 border-b border-[var(--color-border)] text-xs text-[color:var(--color-text-secondary)]">
              <div>OS: {platformInfo.os}</div>
              <div>Arch: {platformInfo.arch}</div>
              <div>Version: {platformInfo.version}</div>
            </div>
          )}
          <div className="px-3 py-2 space-y-2 max-h-48 overflow-y-auto">
            {Object.keys(configEntries).length > 0 ? (
              Object.entries(configEntries).map(([key, val]) => (
                <div key={key} className="flex items-center justify-between text-xs">
                  <span className="font-mono text-[color:var(--color-text-secondary)]">{key}</span>
                  <span className="font-mono text-[color:var(--color-text-tertiary)] truncate max-w-[140px]" title={val}>{val}</span>
                </div>
              ))
            ) : (
              <div className="text-xs text-[color:var(--color-text-tertiary)]">No config entries</div>
            )}
          </div>
          <div className="px-3 py-2 border-t border-[var(--color-border)] space-y-1">
            <div className="flex gap-1">
              <input type="text" value={configKey} onChange={(e) => setConfigKey(e.target.value)} className="flex-1 px-2 py-1 text-xs border rounded bg-[var(--color-bg)] border-[var(--color-border)]" placeholder="Key" />
              <input type="text" value={configValue} onChange={(e) => setConfigValue(e.target.value)} className="flex-1 px-2 py-1 text-xs border rounded bg-[var(--color-bg)] border-[var(--color-border)]" placeholder="Value" onKeyDown={(e) => e.key === "Enter" && handleSetConfig()} />
              <button onClick={handleSetConfig} disabled={!configKey.trim()} className="px-2 py-1 text-xs rounded bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] disabled:opacity-50">Set</button>
            </div>
            {configError && (<div className="text-xs text-[color:var(--color-error)]">{configError}</div>)}
          </div>
        </div>
      )}
    </>
  );
}

// ──────────────────────────────────────────────
// Demo data generator
// ──────────────────────────────────────────────

function generateDemoFiles(path: string): FileEntryData[] {
  const segments = path.split("/").filter(Boolean);
  const depth = segments.length;

  const folders = ["Documents", "Downloads", "Pictures", "Music", "Videos", "Desktop", "Projects", "Archives"];
  const files = ["readme.md", "config.json", "index.ts", "styles.css", "package.json", "tsconfig.json", "vite.config.ts", "app.tsx", ".gitignore", ".env"];

  const entries: FileEntryData[] = [];

  const folderCount = Math.max(2, 8 - depth * 2);
  for (let i = 0; i < folderCount; i++) {
    const name = folders[i % folders.length] + (depth > 0 ? `-${depth}` : "");
    entries.push({ name, path: `${path === "/" ? "" : path}/${name}`, is_dir: true, is_symlink: false, size: 0, modified: new Date(Date.now() - i * 86400000).toISOString(), created: new Date(Date.now() - i * 86400000 * 7).toISOString(), is_hidden: false, extension: null, permissions: "755" });
  }

  // Hidden folder for demo
  entries.push({ name: ".config", path: `${path === "/" ? "" : path}/.config`, is_dir: true, is_symlink: false, size: 0, modified: new Date().toISOString(), created: new Date().toISOString(), is_hidden: true, extension: null, permissions: "755" });

  const fileCount = Math.max(3, 12 - depth);
  for (let i = 0; i < fileCount; i++) {
    const name = files[i % files.length];
    const ext = name.split(".").pop() || null;
    entries.push({ name, path: `${path === "/" ? "" : path}/${name}`, is_dir: false, is_symlink: false, size: Math.floor(Math.random() * 1024 * 100) + 100, modified: new Date(Date.now() - i * 3600000).toISOString(), created: new Date(Date.now() - i * 86400000 * 3).toISOString(), is_hidden: name.startsWith("."), extension: ext, permissions: "644" });
  }

  return entries;
}
