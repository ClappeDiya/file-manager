import { useCallback, useRef, useState, useEffect } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { cn } from "@ufop/ui-components";
import { PanelLeftClose, PanelRightClose, Columns2, Rows2 } from "lucide-react";
import { Button } from "@ufop/ui-components";

const MIN_PANE_PERCENT = 20;
const MAX_PANE_PERCENT = 80;
const DIVIDER_WIDTH = 6;

/**
 * Dual-pane file manager layout with draggable divider.
 *
 * Features:
 * - Side-by-side pane layout
 * - Draggable divider for resizing (20%–80% range)
 * - Single-pane toggle
 * - Keyboard shortcut to switch focus between panes (Ctrl+Tab / Cmd+\)
 * - Each pane is independently navigable
 */
export function DualPaneLayout({
  renderPane,
}: {
  renderPane: (paneIndex: 0 | 1) => React.ReactNode;
}) {
  const activePaneIndex = useFileManagerStore((s) => s.activePaneIndex);
  const singlePaneMode = useFileManagerStore((s) => s.singlePaneMode);
  const paneSplitPercent = useFileManagerStore((s) => s.paneSplitPercent);
  const paneOrientation = useFileManagerStore((s) => s.paneOrientation);
  const setActivePaneIndex = useFileManagerStore((s) => s.setActivePaneIndex);
  const toggleSinglePaneMode = useFileManagerStore((s) => s.toggleSinglePaneMode);
  const setPaneSplitPercent = useFileManagerStore((s) => s.setPaneSplitPercent);
  const isVertical = paneOrientation === "vertical";

  const containerRef = useRef<HTMLDivElement>(null);
  const [isDragging, setIsDragging] = useState(false);

  // Switch active pane on keyboard shortcut
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Cmd+\ (Mac) or Ctrl+\ (Windows/Linux) to switch focus
      if ((e.metaKey || e.ctrlKey) && e.key === "\\") {
        e.preventDefault();
        if (!singlePaneMode) {
          setActivePaneIndex(activePaneIndex === 0 ? 1 : 0);
        }
      }
      // Cmd+Shift+D to toggle dual pane
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "D") {
        e.preventDefault();
        toggleSinglePaneMode();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activePaneIndex, singlePaneMode, setActivePaneIndex, toggleSinglePaneMode]);

  // Drag to resize
  const handleDividerMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      setIsDragging(true);

      const container = containerRef.current;
      if (!container) return;

      const containerRect = container.getBoundingClientRect();

      function onMouseMove(ev: MouseEvent) {
        let percent: number;
        if (isVertical) {
          const relY = ev.clientY - containerRect.top;
          percent = (relY / containerRect.height) * 100;
        } else {
          const relX = ev.clientX - containerRect.left;
          percent = (relX / containerRect.width) * 100;
        }
        setPaneSplitPercent(
          Math.max(MIN_PANE_PERCENT, Math.min(MAX_PANE_PERCENT, percent)),
        );
      }

      function onMouseUp() {
        setIsDragging(false);
        window.removeEventListener("mousemove", onMouseMove);
        window.removeEventListener("mouseup", onMouseUp);
      }

      window.addEventListener("mousemove", onMouseMove);
      window.addEventListener("mouseup", onMouseUp);
    },
    [setPaneSplitPercent, isVertical],
  );

  // Keyboard resize on divider
  const handleDividerKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const decreaseKey = isVertical ? "ArrowUp" : "ArrowLeft";
      const increaseKey = isVertical ? "ArrowDown" : "ArrowRight";
      if (e.key === decreaseKey) {
        e.preventDefault();
        setPaneSplitPercent(paneSplitPercent - 2);
      } else if (e.key === increaseKey) {
        e.preventDefault();
        setPaneSplitPercent(paneSplitPercent + 2);
      }
    },
    [paneSplitPercent, setPaneSplitPercent, isVertical],
  );

  if (singlePaneMode) {
    return (
      <div className="flex h-full flex-col" data-testid="dual-pane-layout">
        <PaneToolbar />
        <div
          className={cn(
            "flex-1 overflow-hidden border-2 border-transparent",
            "border-[var(--color-primary)]",
          )}
          onClick={() => setActivePaneIndex(0)}
          data-testid="pane-0"
          role="region"
          aria-label="File pane"
        >
          {renderPane(0)}
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col" data-testid="dual-pane-layout">
      <PaneToolbar />
      <div
        ref={containerRef}
        className={cn(
          "flex flex-1 overflow-hidden",
          isVertical ? "flex-col" : "flex-row",
          isDragging && "select-none",
        )}
      >
        {/* First Pane (Left or Top) */}
        <div
          className={cn(
            "overflow-hidden border-2 transition-theme",
            activePaneIndex === 0
              ? "border-[var(--color-primary)]"
              : "border-transparent",
          )}
          style={
            isVertical
              ? { height: `calc(${paneSplitPercent}% - ${DIVIDER_WIDTH / 2}px)` }
              : { width: `calc(${paneSplitPercent}% - ${DIVIDER_WIDTH / 2}px)` }
          }
          onClick={() => setActivePaneIndex(0)}
          data-testid="pane-0"
          role="region"
          aria-label={isVertical ? "Top file pane" : "Left file pane"}
        >
          {renderPane(0)}
        </div>

        {/* Divider */}
        <div
          className={cn(
            "flex items-center justify-center",
            isVertical ? "cursor-row-resize" : "cursor-col-resize",
            "bg-[var(--color-border)] hover:bg-[var(--color-primary)]",
            "transition-theme",
            isDragging && "bg-[var(--color-primary)]",
          )}
          style={
            isVertical
              ? { height: `${DIVIDER_WIDTH}px`, flexShrink: 0 }
              : { width: `${DIVIDER_WIDTH}px`, flexShrink: 0 }
          }
          onMouseDown={handleDividerMouseDown}
          onKeyDown={handleDividerKeyDown}
          role="separator"
          aria-orientation={isVertical ? "horizontal" : "vertical"}
          aria-valuenow={Math.round(paneSplitPercent)}
          aria-valuemin={MIN_PANE_PERCENT}
          aria-valuemax={MAX_PANE_PERCENT}
          aria-label="Resize panes"
          tabIndex={0}
          data-testid="pane-divider"
        >
          <div className={cn(
            "rounded-full bg-[var(--color-text-tertiary)] opacity-50",
            isVertical ? "w-8 h-1" : "h-8 w-1",
          )} />
        </div>

        {/* Second Pane (Right or Bottom) */}
        <div
          className={cn(
            "overflow-hidden border-2 transition-theme",
            activePaneIndex === 1
              ? "border-[var(--color-primary)]"
              : "border-transparent",
          )}
          style={
            isVertical
              ? { height: `calc(${100 - paneSplitPercent}% - ${DIVIDER_WIDTH / 2}px)` }
              : { width: `calc(${100 - paneSplitPercent}% - ${DIVIDER_WIDTH / 2}px)` }
          }
          onClick={() => setActivePaneIndex(1)}
          data-testid="pane-1"
          role="region"
          aria-label={isVertical ? "Bottom file pane" : "Right file pane"}
        >
          {renderPane(1)}
        </div>
      </div>
    </div>
  );
}

/**
 * Toolbar for pane layout controls.
 */
function PaneToolbar() {
  const singlePaneMode = useFileManagerStore((s) => s.singlePaneMode);
  const toggleSinglePaneMode = useFileManagerStore((s) => s.toggleSinglePaneMode);
  const activePaneIndex = useFileManagerStore((s) => s.activePaneIndex);
  const setActivePaneIndex = useFileManagerStore((s) => s.setActivePaneIndex);
  const paneOrientation = useFileManagerStore((s) => s.paneOrientation);
  const togglePaneOrientation = useFileManagerStore((s) => s.togglePaneOrientation);
  const isVertical = paneOrientation === "vertical";

  return (
    <div
      className="flex items-center gap-1 px-2 py-1 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]"
      role="toolbar"
      aria-label="Pane controls"
      data-testid="pane-toolbar"
    >
      <Button
        variant="ghost"
        size="icon"
        onClick={toggleSinglePaneMode}
        aria-label={singlePaneMode ? "Enable dual-pane mode" : "Enable single-pane mode"}
        title={singlePaneMode ? "Dual Pane (Cmd+Shift+D)" : "Single Pane (Cmd+Shift+D)"}
        data-testid="toggle-pane-mode"
      >
        {singlePaneMode ? (
          <Columns2 className="h-4 w-4" aria-hidden="true" />
        ) : (
          <PanelLeftClose className="h-4 w-4" aria-hidden="true" />
        )}
      </Button>

      {!singlePaneMode && (
        <>
          <Button
            variant="ghost"
            size="icon"
            onClick={togglePaneOrientation}
            aria-label={isVertical ? "Switch to side-by-side layout" : "Switch to top-bottom layout"}
            title={isVertical ? "Side by Side" : "Top / Bottom"}
            data-testid="toggle-pane-orientation"
          >
            {isVertical ? (
              <Columns2 className="h-3.5 w-3.5" aria-hidden="true" />
            ) : (
              <Rows2 className="h-3.5 w-3.5" aria-hidden="true" />
            )}
          </Button>
          <Button
            variant={activePaneIndex === 0 ? "secondary" : "ghost"}
            size="sm"
            onClick={() => setActivePaneIndex(0)}
            aria-label={isVertical ? "Focus top pane" : "Focus left pane"}
            aria-pressed={activePaneIndex === 0}
            data-testid="focus-left-pane"
          >
            <PanelLeftClose className="h-3 w-3 mr-1" aria-hidden="true" />
            {isVertical ? "Top" : "Left"}
          </Button>
          <Button
            variant={activePaneIndex === 1 ? "secondary" : "ghost"}
            size="sm"
            onClick={() => setActivePaneIndex(1)}
            aria-label={isVertical ? "Focus bottom pane" : "Focus right pane"}
            aria-pressed={activePaneIndex === 1}
            data-testid="focus-right-pane"
          >
            {isVertical ? "Bottom" : "Right"}
            <PanelRightClose className="h-3 w-3 ml-1" aria-hidden="true" />
          </Button>
        </>
      )}
    </div>
  );
}
