import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFileManagerStore, type ViewMode } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";
import { useAiStore } from "@/stores/ai-store";
import { useAutomationStore } from "@/stores/automation-store";
import { useSpacesStore } from "@/stores/spaces-store";
import { useTerminalStore } from "@/stores/terminal-store";
import { isTauriAvailable, tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { DualPaneLayout } from "./dual-pane-layout";
import { TabBar } from "./tab-bar";
import { BreadcrumbBar } from "./breadcrumb-bar";
import { SidebarNav } from "./sidebar-nav";
import { FileList, type FileEntryData } from "./file-list";
import { FilterBar, filterFiles } from "./filter-bar";
import { ContextMenu, getFileContextMenuItems, type ContextMenuItem } from "./context-menu";
import { CommandPalette, getDefaultCommands } from "./command-palette";
import { AiPanel } from "./ai-panel";
import { AutomationPanel } from "./automation-panel";
import { SmartSpaceWizard } from "./smart-space-wizard";
import { TerminalPanel } from "./terminal-panel";
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
  { id: "grouping", label: "Grouping" },
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
export function FileManager() {
  const appMode = useUIStore((s) => s.appMode);
  const sidebarOpen = useUIStore((s) => s.sidebarOpen);
  const sidebarWidth = useUIStore((s) => s.sidebarWidth);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const commandPaletteOpen = useFileManagerStore((s) => s.commandPaletteOpen);
  const setCommandPaletteOpen = useFileManagerStore((s) => s.setCommandPaletteOpen);
  const undoStack = useFileManagerStore((s) => s.undoStack);
  const popUndo = useFileManagerStore((s) => s.popUndo);

  // AI panel state (T-043..T-045)
  const aiPanelOpen = useAiStore((s) => s.panelOpen);
  const toggleAiPanel = useAiStore((s) => s.togglePanel);

  // Automation panel (Quickflows)
  const automationPanelOpen = useAutomationStore((s) => s.panelOpen);
  const toggleAutomationPanel = useAutomationStore((s) => s.togglePanel);

  // Smart Spaces
  const spacesWizardOpen = useSpacesStore((s) => s.wizardOpen);
  const closeSpacesWizard = useSpacesStore((s) => s.closeWizard);

  // Terminal state (T-046)
  const terminalPanelOpen = useTerminalStore((s) => s.panelOpen);
  const toggleTerminalPanel = useTerminalStore((s) => s.togglePanel);
  const createLocalSession = useTerminalStore((s) => s.createLocalSession);

  // Sync browsing (#38)
  const syncBrowsing = useFileManagerStore((s) => s.syncBrowsing);
  const toggleSyncBrowsing = useFileManagerStore((s) => s.toggleSyncBrowsing);
  const singlePaneMode = useFileManagerStore((s) => s.singlePaneMode);

  // Toolbar customization (#14)
  const toolbarItems = useUIStore((s) => s.toolbarItems);
  const setToolbarItems = useUIStore((s) => s.setToolbarItems);
  const toolbarCustomizerOpen = useUIStore((s) => s.toolbarCustomizerOpen);
  const toggleToolbarCustomizer = useUIStore((s) => s.toggleToolbarCustomizer);

  // Workspace management
  const workspaces = useFileManagerStore((s) => s.workspaces);
  const saveWorkspace = useFileManagerStore((s) => s.saveWorkspace);
  const loadWorkspace = useFileManagerStore((s) => s.loadWorkspace);
  const deleteWorkspace = useFileManagerStore((s) => s.deleteWorkspace);
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);

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

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    position: { x: number; y: number };
    items: ContextMenuItem[];
  } | null>(null);

  const handleUndo = useCallback(async () => {
    const entry = popUndo();
    if (entry) {
      if (isTauriAvailable()) {
        try {
          await tauriInvoke("undo_file_operation", { operationId: entry.id });
        } catch (err) {
          console.error("Undo failed:", err);
        }
      } else {
        console.log("Undo (demo mode):", entry);
      }
    }
  }, [popUndo]);

  // Global keyboard shortcuts
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
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
  }, [toggleSidebar, undoStack, toggleAiPanel, toggleTerminalPanel, handleUndo, handleSaveWorkspace]);

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

  // Default commands for command palette
  const commands = useMemo(
    () =>
      getDefaultCommands({
        onToggleDualPane: () =>
          useFileManagerStore.getState().toggleSinglePaneMode(),
        onToggleSidebar: toggleSidebar,
        onRefresh: () => {
          window.dispatchEvent(new CustomEvent("ufop:refresh-directory"));
        },
        onUndo: handleUndo,
        onSetTheme: (theme) =>
          useUIStore.getState().setTheme(theme as "light" | "dark" | "system"),
        onSetViewMode: (mode) => {
          const store = useFileManagerStore.getState();
          store.setViewMode(store.activePaneIndex, mode as ViewMode);
        },
        onSaveWorkspace: handleSaveWorkspace,
        onLoadWorkspace: () => setWorkspaceMenuOpen(true),
        onCopyPath: async () => {
          const path = getFirstSelectedPath();
          if (path) {
            try { await navigator.clipboard.writeText(path); } catch {}
          }
        },
        onCopyRemotePath: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          try {
            const result = await tauriInvoke<{ path: string }>("copy_remote_path", { path, remoteBase: null });
            await navigator.clipboard.writeText(result.path);
          } catch {}
        },
        onCopyUrl: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          try {
            const result = await tauriInvoke<{ path: string }>("copy_url", { path, protocol: "local", host: "localhost", port: null, username: null });
            await navigator.clipboard.writeText(result.path);
          } catch {}
        },
        onCopyRelativePath: async () => {
          const path = getFirstSelectedPath();
          if (!path) return;
          const store = useFileManagerStore.getState();
          const basePath = store.getActivePath(store.activePaneIndex);
          try {
            const result = await tauriInvoke<{ path: string }>("copy_relative_path", { path, basePath });
            await navigator.clipboard.writeText(result.path);
          } catch {}
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
      }),
    [toggleSidebar, handleUndo, handleSaveWorkspace, getFirstSelectedPath, toggleAutomationPanel, automationPanelOpen],
  );

  return (
    <div className="flex h-full flex-col" data-testid="file-manager">
      {/* Toolbar */}
      <header
        className="relative flex items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-toolbar-bg)] px-4"
        style={{ height: "var(--toolbar-height)" }}
        role="toolbar"
        aria-label="Main toolbar"
        onContextMenu={(e) => { e.preventDefault(); toggleToolbarCustomizer(); }}
      >
        <div className="flex items-center gap-3">
          {toolbarItems.includes("sidebar") && (
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
          <FolderOpen className="h-5 w-5 text-[color:var(--color-primary)]" aria-hidden="true" />
          <h1 className="text-[length:var(--font-size-md)] font-semibold text-[color:var(--color-text)]">
            File Manager
          </h1>
          <Badge variant="secondary">{appMode}</Badge>
        </div>

        <div className="flex items-center gap-2">
          {toolbarItems.includes("view-modes") && <ViewModeSelector />}
          {toolbarItems.includes("hidden-files") && <HiddenFilesToggle />}
          {/* Sync Browsing toggle (#38) */}
          {!singlePaneMode && (
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
          {syncBrowsing && !singlePaneMode && (
            <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-primary)] font-medium">Sync</span>
          )}
          {undoStack.length > 0 && (
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
              title="Workspaces"
              data-testid="workspace-menu-trigger"
            >
              <FolderDown className="h-4 w-4" aria-hidden="true" />
            </Button>
            {workspaceMenuOpen && (
              <div className="absolute right-0 top-full z-50 mt-1 w-64 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-lg overflow-hidden">
                <div className="px-3 py-2 border-b border-[var(--color-border)] flex items-center justify-between">
                  <span className="text-xs font-semibold text-[color:var(--color-text)]">Workspaces</span>
                  <button
                    onClick={() => { handleSaveWorkspace(); }}
                    className="flex items-center gap-1 text-xs text-blue-500 hover:text-blue-600"
                  >
                    <Save className="h-3 w-3" />
                    Save Current
                  </button>
                </div>
                <div className="max-h-48 overflow-auto">
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
                      >
                        <button
                          onClick={() => handleLoadWorkspace(ws.id)}
                          className="flex-1 text-left text-xs text-[color:var(--color-text)] truncate"
                          title={`Load "${ws.name}" (saved ${new Date(ws.savedAt).toLocaleDateString()})`}
                        >
                          {ws.name}
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleDeleteWorkspace(ws.id); }}
                          className="text-xs text-red-400 hover:text-red-500 opacity-0 group-hover:opacity-100 ml-2"
                          title="Delete workspace"
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
          {toolbarItems.includes("theme") && <ThemeSwitcher />}
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
            <button
              onClick={toggleToolbarCustomizer}
              className="mt-2 w-full text-xs py-1 rounded bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]"
            >
              Done
            </button>
          </div>
        )}
      </header>

      {/* Main content area */}
      <div className="flex flex-1 overflow-hidden">
        {sidebarOpen && (
          <aside
            className="border-r border-[var(--color-border)] shrink-0 overflow-hidden"
            style={{ width: sidebarWidth }}
            role="navigation"
            aria-label="File navigation sidebar"
          >
            <SidebarNav
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
                <FilePane paneIndex={paneIndex} onContextMenu={setContextMenu} />
              )}
            />
          </div>
          <TerminalPanel />
        </div>

        <AutomationPanel />
        <AiPanel />
      </div>

      {/* Smart Space Wizard */}
      {spacesWizardOpen && <SmartSpaceWizard onClose={closeSpacesWizard} />}

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
      />
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
}

function FilePane({ paneIndex, onContextMenu }: FilePaneProps) {
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

  // Feature 2: Git status (#46)
  const [gitStatus, setGitStatus] = useState<Record<string, string>>({});
  const [isGitRepo, setIsGitRepo] = useState(false);

  // Feature 5: Editable comments (#82)
  const [infoDialog, setInfoDialog] = useState<{
    path: string;
    name: string;
    metadata: Record<string, any> | null;
    comment: string;
  } | null>(null);

  // Feature 2: Folder size calculation (#83)
  const [_folderSizes, setFolderSizes] = useState<Record<string, number>>({});

  // Feature 3: Quick Select (#71)
  const [quickSelectOpen, setQuickSelectOpen] = useState(false);
  const [quickSelectPattern, setQuickSelectPattern] = useState("");

  // Compare files state (#47)
  const [_compareData, setCompareData] = useState<{left: string, right: string, leftName: string, rightName: string} | null>(null);

  // Feature 5: Per-folder view defaults debounce timer (#13)
  const folderViewSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Load directory contents ──

  const loadDirectory = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
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

  // Listen for refresh events
  useEffect(() => {
    const handler = () => loadDirectory(currentPath);
    window.addEventListener("ufop:refresh-directory", handler);
    return () => window.removeEventListener("ufop:refresh-directory", handler);
  }, [currentPath, loadDirectory]);

  // Apply filter
  const filteredFiles = useMemo(
    () => filterFiles(files, pane.filterText),
    [files, pane.filterText],
  );

  // Feature 1: Hidden file filtering (#85)
  const visibleFiles = useMemo(() => {
    if (showHiddenFiles) return filteredFiles;
    return filteredFiles.filter(f => !f.is_hidden && !f.name.startsWith('.'));
  }, [filteredFiles, showHiddenFiles]);

  // Selection
  const { selectedPaths, handleSelect, selectAll, invertSelection } =
    useFileSelection(pane.id, visibleFiles);

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

  // Open file/folder
  const handleOpen = useCallback(
    (entry: FileEntryData) => {
      if (entry.is_dir) {
        handleNavigate(entry.path);
      } else if (isTauriAvailable()) {
        tauriInvoke("open_file_with_default", { path: entry.path }).catch((err) => {
          console.error("Failed to open file:", err);
        });
      }
    },
    [handleNavigate],
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
    setFolderSizes(prev => ({ ...prev, [folderPath]: size }));
    console.log(`Folder size for ${folderPath}: ${size} bytes`);
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
        window.dispatchEvent(new CustomEvent("ufop:refresh-directory"));
      } catch (err) { console.error("Copy failed:", err); }
    } else { console.log("Copy (demo mode):", selectedPaths); }
  }, [selectedPaths, paneIndex, pushUndo, checkConflicts]);

  const handleCut = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        const store = useFileManagerStore.getState();
        const otherPaneIndex = paneIndex === 0 ? 1 : 0;
        const otherPath = store.getActivePath(otherPaneIndex);
        const proceed = await checkConflicts(selectedPaths, otherPath);
        if (!proceed) return;
        await tauriInvoke("move_files", { sourcePaths: selectedPaths, destDir: otherPath });
        pushUndo({ id: `move-${Date.now()}`, type: "move", sourcePaths: selectedPaths, destPaths: selectedPaths.map((p) => `${otherPath}/${p.split("/").pop()}`), timestamp: Date.now() });
        loadDirectory(currentPath);
      } catch (err) { console.error("Move failed:", err); }
    } else { console.log("Cut (demo mode):", selectedPaths); }
  }, [selectedPaths, paneIndex, pushUndo, loadDirectory, currentPath, checkConflicts]);

  const handleDelete = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        await tauriInvoke("delete_files", { paths: selectedPaths, permanent: false });
        pushUndo({ id: `delete-${Date.now()}`, type: "delete", sourcePaths: selectedPaths, destPaths: [], timestamp: Date.now() });
        loadDirectory(currentPath);
      } catch (err) { console.error("Delete failed:", err); }
    } else { console.log("Delete (demo mode):", selectedPaths); }
  }, [selectedPaths, pushUndo, loadDirectory, currentPath]);

  const handleDeletePermanently = useCallback(async (paths: string[]) => {
    if (paths.length === 0) return;
    const fileNames = paths.map((p) => p.split("/").pop()).join(", ");
    const confirmed = window.confirm(
      `Permanently delete ${paths.length} item(s)?\n\n${fileNames}\n\nThis action cannot be undone.`
    );
    if (!confirmed) return;
    if (isTauriAvailable()) {
      try {
        // Use cloud_delete_permanently for cloud provider files, otherwise delete_files with permanent=true
        try {
          await tauriInvoke("cloud_delete_permanently", { paths });
        } catch {
          // Fallback to local permanent delete if cloud command is not available
          await tauriInvoke("delete_files", { paths, permanent: true });
        }
        loadDirectory(currentPath);
      } catch (err) { console.error("Permanent delete failed:", err); }
    } else { console.log("Delete permanently (demo mode):", paths); }
  }, [loadDirectory, currentPath]);

  const handleRename = useCallback(async (filePath: string) => {
    if (isTauriAvailable()) {
      const oldName = filePath.split("/").pop() || "";
      const newName = window.prompt("Enter new name:", oldName);
      if (newName && newName !== oldName) {
        try {
          await tauriInvoke("rename_file", { sourcePath: filePath, newName });
          pushUndo({ id: `rename-${Date.now()}`, type: "rename", sourcePaths: [filePath], destPaths: [filePath.replace(/[^/]+$/, newName)], timestamp: Date.now() });
          loadDirectory(currentPath);
        } catch (err) { console.error("Rename failed:", err); }
      }
    } else { console.log("Rename (demo mode):", filePath); }
  }, [pushUndo, loadDirectory, currentPath]);

  const handleDuplicate = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        await tauriInvoke("duplicate_files", { paths: selectedPaths });
        loadDirectory(currentPath);
      } catch (err) { console.error("Duplicate failed:", err); }
    } else { console.log("Duplicate (demo mode):", selectedPaths); }
  }, [selectedPaths, loadDirectory, currentPath]);

  const handleNewFolder = useCallback(async () => {
    if (isTauriAvailable()) {
      const name = window.prompt("New folder name:", "New Folder");
      if (name) {
        try {
          await tauriInvoke("create_directory", { path: `${currentPath === "/" ? "" : currentPath}/${name}` });
          pushUndo({ id: `mkdir-${Date.now()}`, type: "create_folder", sourcePaths: [], destPaths: [`${currentPath === "/" ? "" : currentPath}/${name}`], timestamp: Date.now() });
          loadDirectory(currentPath);
        } catch (err) { console.error("Create directory failed:", err); }
      }
    } else { console.log("New Folder (demo mode) in", currentPath); }
  }, [currentPath, pushUndo, loadDirectory]);

  const handleNewFile = useCallback(async () => {
    if (isTauriAvailable()) {
      const name = window.prompt("New file name:", "untitled.txt");
      if (name) {
        try {
          await tauriInvoke("create_file", { path: `${currentPath === "/" ? "" : currentPath}/${name}` });
          pushUndo({ id: `mkfile-${Date.now()}`, type: "create_file", sourcePaths: [], destPaths: [`${currentPath === "/" ? "" : currentPath}/${name}`], timestamp: Date.now() });
          loadDirectory(currentPath);
        } catch (err) { console.error("Create file failed:", err); }
      }
    } else { console.log("New File (demo mode) in", currentPath); }
  }, [currentPath, pushUndo, loadDirectory]);

  const handleGetInfo = useCallback(async (filePath: string) => {
    const fileName = filePath.split("/").pop() || filePath;
    let metadata: Record<string, any> | null = null;
    let comment = "";

    if (isTauriAvailable()) {
      try {
        metadata = await tauriInvoke("get_file_metadata", { path: filePath });
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
        loadDirectory(currentPath);
      } catch (err) {
        console.error("Set permissions failed:", err);
      }
    } else {
      console.log("Set permissions (demo mode):", selectedPaths, mode);
    }
  }, [selectedPaths, loadDirectory, currentPath]);

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
        await tauriInvoke("create_symlink", {
          target,
          linkPath: linkName.trim(),
        });
        loadDirectory(currentPath);
      } catch (err) {
        console.error("Create symlink failed:", err);
      }
    } else {
      console.log("Create symlink (demo mode):", target, "->", linkName);
    }
  }, [selectedPaths, loadDirectory, currentPath]);

  // Copy as script handler
  const handleCopyAsScript = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    if (isTauriAvailable()) {
      try {
        const script = await tauriInvoke<string>("generate_script", {
          operation: "copy",
          sourcePaths: selectedPaths,
          destPath: null,
          options: null,
        });
        await navigator.clipboard.writeText(script);
      } catch (err) {
        console.error("Copy as script failed:", err);
      }
    } else {
      const script = selectedPaths
        .map((p) => `cp -r "${p}" .`)
        .join("\n");
      await navigator.clipboard.writeText(script);
      console.log("Copy as script (demo mode):", script);
    }
  }, [selectedPaths]);

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

  // Copy Path handlers
  const handleCopyPath = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    const path = selectedPaths[0];
    try {
      await navigator.clipboard.writeText(path);
    } catch {}
  }, [selectedPaths]);

  const handleCopyRemotePath = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    const path = selectedPaths[0];
    try {
      const result = await tauriInvoke<{ path: string }>("copy_remote_path", {
        path,
        remoteBase: null,
      });
      await navigator.clipboard.writeText(result.path);
    } catch {}
  }, [selectedPaths]);

  const handleCopyUrl = useCallback(async () => {
    if (selectedPaths.length === 0) return;
    const path = selectedPaths[0];
    try {
      const result = await tauriInvoke<{ path: string }>("copy_url", {
        path,
        protocol: "local",
        host: "localhost",
        port: null,
        username: null,
      });
      await navigator.clipboard.writeText(result.path);
    } catch {}
  }, [selectedPaths]);

  const handleCopyRelativePath = useCallback(async () => {
    const basePath = currentPath;
    if (selectedPaths.length === 0) return;
    const path = selectedPaths[0];
    try {
      const result = await tauriInvoke<{ path: string }>("copy_relative_path", {
        path,
        basePath,
      });
      await navigator.clipboard.writeText(result.path);
    } catch {}
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

  // Context menu
  const handleContextMenu = useCallback(
    (entry: FileEntryData, event: React.MouseEvent) => {
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
      });
      onContextMenu({ position: { x: event.clientX, y: event.clientY }, items });
    },
    [selectedPaths, currentPath, handleOpen, handleCopy, handleCut, handleDelete, handleDeletePermanently, handleRename, handleDuplicate, handleNewFolder, handleNewFile, handleGetInfo, handleCalculateSize, handleEditRemote, handleCompareFiles, handleSetPermissions, handleCreateSymlink, handleCopyAsScript, handleCopyPath, handleCopyRemotePath, handleCopyUrl, handleCopyRelativePath, handleOpenWith, handleOpenInEditor, selectAll, invertSelection, onContextMenu, loadDirectory, isGitRepo],
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
              .then(() => { loadDirectory(currentPath); })
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
        groupBy={pane.groupBy}
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

/** Stable empty array to avoid re-render loops with React 19 + Zustand */
const EMPTY_PATHS: string[] = [];

function StatusBarContent() {
  const selectedPaths = useFileManagerStore(
    (s) => s.selectedPaths[s.panes[s.activePaneIndex].id] ?? EMPTY_PATHS,
  );

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
      <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
        {selectedPaths.length > 0
          ? `${selectedPaths.length} item${selectedPaths.length !== 1 ? "s" : ""} selected`
          : "Ready"}
      </span>
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
