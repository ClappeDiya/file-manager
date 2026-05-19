import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFileManagerStore, type PaneTab } from "@/stores/file-manager-store";
import { cn } from "@ufop/ui-components";
import { Tooltip } from "@ufop/ui-components";
import { X, Plus, Pin, PinOff } from "lucide-react";
import { ContextMenu, getTabContextMenuItems } from "./context-menu";

/**
 * Tabbed browsing bar for a single pane.
 * Supports:
 * - New tab (Cmd+T)
 * - Close tab (click X or Cmd+W)
 * - Pin/unpin tabs
 * - Drag to reorder
 * - Tab shows folder name with full path tooltip
 * - Keyboard navigation (Ctrl+Tab to switch tabs)
 */
interface TabBarProps {
  paneIndex: 0 | 1;
  className?: string;
}

export function TabBar({ paneIndex, className }: TabBarProps) {
  const pane = useFileManagerStore((s) => s.panes[paneIndex]);
  const addTab = useFileManagerStore((s) => s.addTab);
  const closeTab = useFileManagerStore((s) => s.closeTab);
  const setActiveTab = useFileManagerStore((s) => s.setActiveTab);
  const pinTab = useFileManagerStore((s) => s.pinTab);
  const unpinTab = useFileManagerStore((s) => s.unpinTab);
  const reorderTabs = useFileManagerStore((s) => s.reorderTabs);
  const activePaneIndex = useFileManagerStore((s) => s.activePaneIndex);
  // Iter 20: tab context menu actions
  const closeOtherTabs = useFileManagerStore((s) => s.closeOtherTabs);
  const closeTabsToRight = useFileManagerStore((s) => s.closeTabsToRight);
  const duplicateTab = useFileManagerStore((s) => s.duplicateTab);
  const moveTabToOtherPane = useFileManagerStore((s) => s.moveTabToOtherPane);

  const tabsRef = useRef<HTMLDivElement>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);
  // Iter 20: tab context menu state. `null` when closed; carries the
  // anchor tab id + click position when open. Mirrors the file
  // context menu pattern in file-manager.tsx: parent holds state,
  // <ContextMenu> renders once at the TabBar root.
  const [tabMenu, setTabMenu] = useState<{
    tabId: string;
    x: number;
    y: number;
  } | null>(null);

  // Global keyboard shortcuts
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Only respond when this pane is active
      if (activePaneIndex !== paneIndex) return;

      // Cmd+T / Ctrl+T - New tab
      if ((e.metaKey || e.ctrlKey) && e.key === "t") {
        e.preventDefault();
        const currentTab = pane.tabs.find((t) => t.id === pane.activeTabId);
        const path = currentTab?.path || "/";
        const label = path === "/" ? "Root" : path.split("/").filter(Boolean).pop() || "Root";
        addTab(paneIndex, path, label);
      }

      // Cmd+W / Ctrl+W - Close tab
      if ((e.metaKey || e.ctrlKey) && e.key === "w") {
        e.preventDefault();
        if (pane.tabs.length > 1) {
          closeTab(paneIndex, pane.activeTabId);
        }
      }

      // Ctrl+Tab / Ctrl+Shift+Tab - Switch tabs
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        const currentIdx = pane.tabs.findIndex((t) => t.id === pane.activeTabId);
        if (e.shiftKey) {
          const prevIdx = (currentIdx - 1 + pane.tabs.length) % pane.tabs.length;
          setActiveTab(paneIndex, pane.tabs[prevIdx].id);
        } else {
          const nextIdx = (currentIdx + 1) % pane.tabs.length;
          setActiveTab(paneIndex, pane.tabs[nextIdx].id);
        }
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [paneIndex, activePaneIndex, pane, addTab, closeTab, setActiveTab]);

  // Drag handlers
  const handleDragStart = useCallback((e: React.DragEvent, index: number) => {
    setDragIndex(index);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(index));
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent, index: number) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setDropIndex(index);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent, toIndex: number) => {
      e.preventDefault();
      if (dragIndex !== null && dragIndex !== toIndex) {
        reorderTabs(paneIndex, dragIndex, toIndex);
      }
      setDragIndex(null);
      setDropIndex(null);
    },
    [dragIndex, paneIndex, reorderTabs],
  );

  const handleDragEnd = useCallback(() => {
    setDragIndex(null);
    setDropIndex(null);
  }, []);

  const handleNewTab = useCallback(() => {
    const currentTab = pane.tabs.find((t) => t.id === pane.activeTabId);
    const path = currentTab?.path || "/";
    const label = path === "/" ? "Root" : path.split("/").filter(Boolean).pop() || "Root";
    addTab(paneIndex, path, label);
  }, [pane, paneIndex, addTab]);

  // Iter 20: compute the right-click menu items lazily, only when the
  // menu is open. Counts and the target tab snapshot are derived from
  // the current tabs list at menu-build time, so they stay in sync
  // with any close/pin that happened between the right-click and the
  // user picking an action.
  const tabMenuItems = useMemo(() => {
    if (!tabMenu) return [];
    const target = pane.tabs.find((t) => t.id === tabMenu.tabId);
    if (!target) return [];

    const sorted = [...pane.tabs].sort((a, b) => {
      if (a.pinned && !b.pinned) return -1;
      if (!a.pinned && b.pinned) return 1;
      return 0;
    });
    const fromIdx = sorted.findIndex((t) => t.id === tabMenu.tabId);
    const closableOthers = pane.tabs.filter(
      (t) => t.id !== tabMenu.tabId && !t.pinned,
    ).length;
    const closableToRight =
      fromIdx >= 0
        ? sorted.slice(fromIdx + 1).filter((t) => !t.pinned).length
        : 0;

    return getTabContextMenuItems({
      isPinned: target.pinned,
      totalTabs: pane.tabs.length,
      closableOthers,
      closableToRight,
      onClose: () => closeTab(paneIndex, tabMenu.tabId),
      onCloseOthers: () => closeOtherTabs(paneIndex, tabMenu.tabId),
      onCloseToRight: () => closeTabsToRight(paneIndex, tabMenu.tabId),
      onPinToggle: () =>
        target.pinned
          ? unpinTab(paneIndex, tabMenu.tabId)
          : pinTab(paneIndex, tabMenu.tabId),
      onDuplicate: () => duplicateTab(paneIndex, tabMenu.tabId),
      onMoveToOtherPane: () =>
        moveTabToOtherPane(paneIndex, tabMenu.tabId),
    });
  }, [
    tabMenu,
    pane.tabs,
    paneIndex,
    closeTab,
    closeOtherTabs,
    closeTabsToRight,
    pinTab,
    unpinTab,
    duplicateTab,
    moveTabToOtherPane,
  ]);

  return (
    <>
    <div
      className={cn(
        "flex items-center gap-0.5 bg-[var(--color-bg-secondary)] border-b border-[var(--color-border)] px-1",
        "overflow-x-auto scrollbar-none",
        className,
      )}
      role="tablist"
      aria-label={`Pane ${paneIndex + 1} tabs`}
      ref={tabsRef}
      data-testid={`tab-bar-${paneIndex}`}
    >
      {/* Pinned tabs come first */}
      {pane.tabs
        .map((tab, index) => ({ tab, index }))
        .sort((a, b) => {
          if (a.tab.pinned && !b.tab.pinned) return -1;
          if (!a.tab.pinned && b.tab.pinned) return 1;
          return 0;
        })
        .map(({ tab, index }) => (
          <TabItem
            key={tab.id}
            tab={tab}
            index={index}
            isActive={tab.id === pane.activeTabId}
            paneIndex={paneIndex}
            onActivate={() => setActiveTab(paneIndex, tab.id)}
            onClose={() => closeTab(paneIndex, tab.id)}
            onPin={() => (tab.pinned ? unpinTab(paneIndex, tab.id) : pinTab(paneIndex, tab.id))}
            onContextMenu={(e) => {
              e.preventDefault();
              setTabMenu({ tabId: tab.id, x: e.clientX, y: e.clientY });
            }}
            canClose={pane.tabs.length > 1}
            isDragging={dragIndex === index}
            isDropTarget={dropIndex === index}
            onDragStart={(e) => handleDragStart(e, index)}
            onDragOver={(e) => handleDragOver(e, index)}
            onDrop={(e) => handleDrop(e, index)}
            onDragEnd={handleDragEnd}
          />
        ))}

      {/* New tab button */}
      <button
        className={cn(
          "flex items-center justify-center h-7 w-7 rounded-[var(--radius-sm)]",
          "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={handleNewTab}
        aria-label="New tab"
        title="New tab (Cmd+T)"
        data-testid="new-tab-button"
      >
        <Plus className="h-3.5 w-3.5" aria-hidden="true" />
      </button>
    </div>

    <ContextMenu
      items={tabMenuItems}
      position={tabMenu ? { x: tabMenu.x, y: tabMenu.y } : null}
      onClose={() => setTabMenu(null)}
    />
    </>
  );
}

// ──────────────────────────────────────────────
// Individual tab item
// ──────────────────────────────────────────────

interface TabItemProps {
  tab: PaneTab;
  index: number;
  isActive: boolean;
  paneIndex: 0 | 1;
  onActivate: () => void;
  onClose: () => void;
  onPin: () => void;
  /** Iter 20 — right-click handler. The parent TabBar handles
   *  preventDefault + opening the shared <ContextMenu>; this prop
   *  just routes the event up. */
  onContextMenu: (e: React.MouseEvent) => void;
  canClose: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  onDragStart: (e: React.DragEvent) => void;
  onDragOver: (e: React.DragEvent) => void;
  onDrop: (e: React.DragEvent) => void;
  onDragEnd: () => void;
}

function TabItem({
  tab,
  isActive,
  onActivate,
  onClose,
  onPin,
  onContextMenu,
  canClose,
  isDragging,
  isDropTarget,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
}: TabItemProps) {
  return (
    <Tooltip content={tab.path}>
      <div
        role="tab"
        aria-selected={isActive}
        className={cn(
          "group flex items-center gap-1 h-8 px-2 rounded-t-[var(--radius-sm)]",
          "text-[length:var(--font-size-xs)] cursor-pointer select-none",
          "transition-theme min-w-0 max-w-[180px]",
          isActive
            ? "bg-[var(--color-bg)] text-[color:var(--color-text)] border-b-2 border-[var(--color-primary)]"
            : "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
          isDragging && "opacity-50",
          isDropTarget && "border-l-2 border-l-[var(--color-primary)]",
        )}
        onClick={onActivate}
        onContextMenu={onContextMenu}
        draggable
        onDragStart={onDragStart}
        onDragOver={onDragOver}
        onDrop={onDrop}
        onDragEnd={onDragEnd}
        data-testid={`tab-${tab.id}`}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onActivate();
          }
        }}
        tabIndex={0}
      >
        {/* Pin indicator */}
        {tab.pinned && (
          <Pin className="h-3 w-3 text-[color:var(--color-primary)] shrink-0" aria-label="Pinned" />
        )}

        {/* Tab label */}
        <span className="truncate" title={tab.path}>
          {tab.label}
        </span>

        {/* Close / pin button on hover */}
        <div className="flex items-center shrink-0 opacity-0 group-hover:opacity-100 transition-theme">
          {/* Pin/unpin button */}
          <button
            className={cn(
              "h-4 w-4 rounded-sm flex items-center justify-center",
              "hover:bg-[var(--color-hover-bg)] text-[color:var(--color-text-tertiary)]",
            )}
            onClick={(e) => {
              e.stopPropagation();
              onPin();
            }}
            aria-label={tab.pinned ? "Unpin tab" : "Pin tab"}
            title={tab.pinned ? "Unpin tab" : "Pin tab"}
          >
            {tab.pinned ? (
              <PinOff className="h-2.5 w-2.5" aria-hidden="true" />
            ) : (
              <Pin className="h-2.5 w-2.5" aria-hidden="true" />
            )}
          </button>

          {/* Close button */}
          {canClose && !tab.pinned && (
            <button
              className={cn(
                "h-4 w-4 rounded-sm flex items-center justify-center",
                "hover:bg-[var(--color-error-bg)] hover:text-[color:var(--color-error)]",
                "text-[color:var(--color-text-tertiary)]",
              )}
              onClick={(e) => {
                e.stopPropagation();
                onClose();
              }}
              aria-label={`Close ${tab.label} tab`}
              title="Close tab (Cmd+W)"
            >
              <X className="h-2.5 w-2.5" aria-hidden="true" />
            </button>
          )}
        </div>
      </div>
    </Tooltip>
  );
}
