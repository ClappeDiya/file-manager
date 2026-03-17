import { useCallback } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";
import type { FileEntryData } from "@/components/file-list";

/** Stable empty array to avoid re-render loops with React 19 + Zustand */
const EMPTY_PATHS: string[] = [];

/**
 * Hook for managing file selection with multi-select support.
 *
 * Supports:
 * - Click: select single item
 * - Shift+Click: range select
 * - Cmd/Ctrl+Click: toggle selection
 * - Cmd/Ctrl+A: select all
 * - Cmd/Ctrl+Shift+I: invert selection
 */
export function useFileSelection(paneId: string, files: FileEntryData[]) {
  const selectedPaths = useFileManagerStore((s) => s.selectedPaths[paneId] ?? EMPTY_PATHS);
  const setSelection = useFileManagerStore((s) => s.setSelection);
  const addToSelection = useFileManagerStore((s) => s.addToSelection);
  const removeFromSelection = useFileManagerStore((s) => s.removeFromSelection);
  const clearSelection = useFileManagerStore((s) => s.clearSelection);

  /**
   * Handle click-based selection.
   * Implements standard OS-like selection behavior.
   */
  const handleSelect = useCallback(
    (path: string, event: React.MouseEvent) => {
      const isMetaKey = event.metaKey || event.ctrlKey;
      const isShiftKey = event.shiftKey;

      if (isMetaKey && isShiftKey) {
        // Cmd+Shift+Click: no special action (reserved for invert)
        return;
      }

      if (isMetaKey) {
        // Cmd/Ctrl+Click: toggle individual item
        if (selectedPaths.includes(path)) {
          removeFromSelection(paneId, path);
        } else {
          addToSelection(paneId, path);
        }
      } else if (isShiftKey) {
        // Shift+Click: range select
        if (selectedPaths.length === 0) {
          setSelection(paneId, [path]);
        } else {
          const lastSelected = selectedPaths[selectedPaths.length - 1];
          const allPaths = files.map((f) => f.path);
          const startIdx = allPaths.indexOf(lastSelected);
          const endIdx = allPaths.indexOf(path);

          if (startIdx !== -1 && endIdx !== -1) {
            const min = Math.min(startIdx, endIdx);
            const max = Math.max(startIdx, endIdx);
            const rangePaths = allPaths.slice(min, max + 1);

            // Merge with existing selection
            const merged = new Set([...selectedPaths, ...rangePaths]);
            setSelection(paneId, [...merged]);
          } else {
            setSelection(paneId, [path]);
          }
        }
      } else {
        // Normal click: select single
        setSelection(paneId, [path]);
      }
    },
    [paneId, selectedPaths, files, setSelection, addToSelection, removeFromSelection],
  );

  /**
   * Select all files in the list.
   */
  const selectAll = useCallback(() => {
    setSelection(
      paneId,
      files.map((f) => f.path),
    );
  }, [paneId, files, setSelection]);

  /**
   * Invert the current selection.
   */
  const invertSelection = useCallback(() => {
    const selectedSet = new Set(selectedPaths);
    const inverted = files.filter((f) => !selectedSet.has(f.path)).map((f) => f.path);
    setSelection(paneId, inverted);
  }, [paneId, selectedPaths, files, setSelection]);

  /**
   * Clear all selections.
   */
  const deselectAll = useCallback(() => {
    clearSelection(paneId);
  }, [paneId, clearSelection]);

  return {
    selectedPaths,
    handleSelect,
    selectAll,
    invertSelection,
    deselectAll,
    selectionCount: selectedPaths.length,
  };
}

/**
 * Hook for drag-and-drop between panes.
 * Sets up drag data and handles drops.
 */
export function useFileDragDrop(paneId: string) {
  const selectedPaths = useFileManagerStore((s) => s.selectedPaths[paneId] ?? EMPTY_PATHS);

  /**
   * Setup drag start for selected files.
   */
  const handleDragStart = useCallback(
    (e: React.DragEvent, entry: FileEntryData) => {
      // Include the dragged item in selection if not already selected
      const paths = selectedPaths.includes(entry.path)
        ? selectedPaths
        : [entry.path];

      e.dataTransfer.setData("application/x-ufop-files", JSON.stringify(paths));
      e.dataTransfer.setData("text/path", entry.path);
      e.dataTransfer.setData("text/name", entry.name);
      e.dataTransfer.effectAllowed = "copyMove";

      // Visual feedback: show count
      if (paths.length > 1) {
        const dragImage = document.createElement("div");
        dragImage.textContent = `${paths.length} items`;
        dragImage.style.cssText =
          "position:absolute;top:-100px;padding:4px 12px;background:#333;color:#fff;border-radius:4px;font-size:12px;";
        document.body.appendChild(dragImage);
        e.dataTransfer.setDragImage(dragImage, 0, 0);
        setTimeout(() => document.body.removeChild(dragImage), 0);
      }
    },
    [selectedPaths],
  );

  /**
   * Handle drag over (allow drop).
   */
  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    const hasFiles = e.dataTransfer.types.includes("application/x-ufop-files");
    e.dataTransfer.dropEffect = hasFiles
      ? e.altKey
        ? "copy"
        : "move"
      : "none";
  }, []);

  /**
   * Handle drop - returns the paths that were dropped.
   */
  const handleDrop = useCallback((e: React.DragEvent): string[] | null => {
    e.preventDefault();
    const data = e.dataTransfer.getData("application/x-ufop-files");
    if (data) {
      try {
        return JSON.parse(data) as string[];
      } catch {
        return null;
      }
    }
    return null;
  }, []);

  return {
    handleDragStart,
    handleDragOver,
    handleDrop,
  };
}
