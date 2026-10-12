import { useCallback, useRef } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";
import type { FileEntryData } from "@/components/file-list";

/** Stable empty array to avoid re-render loops with React 19 + Zustand */
const EMPTY_PATHS: string[] = [];

/**
 * Inclusive range between `anchor` and `target` in list order. Returns just
 * `[target]` when either end is not in the list (e.g. after navigation).
 */
export function computeRangeSelection(allPaths: string[], anchor: string | null, target: string): string[] {
  const a = anchor === null ? -1 : allPaths.indexOf(anchor);
  const b = allPaths.indexOf(target);
  if (a === -1 || b === -1) return [target];
  return allPaths.slice(Math.min(a, b), Math.max(a, b) + 1);
}

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
  // Range selections grow from a fixed anchor (the last plainly-clicked /
  // keyboard-selected item) and REPLACE the previous range, like Finder.
  const anchorRef = useRef<string | null>(null);

  const handleSelect = useCallback(
    (path: string, event: React.MouseEvent) => {
      const isMetaKey = event.metaKey || event.ctrlKey;
      const isShiftKey = event.shiftKey;

      if (isMetaKey && isShiftKey) {
        // Cmd+Shift+Click: no special action (reserved for invert)
        return;
      }

      if (isMetaKey) {
        anchorRef.current = path;
        if (selectedPaths.includes(path)) {
          removeFromSelection(paneId, path);
        } else {
          addToSelection(paneId, path);
        }
      } else if (isShiftKey) {
        const anchor = anchorRef.current ?? selectedPaths[selectedPaths.length - 1] ?? null;
        if (anchorRef.current === null) anchorRef.current = anchor ?? path;
        setSelection(paneId, computeRangeSelection(files.map((f) => f.path), anchor, path));
      } else {
        anchorRef.current = path;
        setSelection(paneId, [path]);
      }
    },
    [paneId, selectedPaths, files, setSelection, addToSelection, removeFromSelection],
  );

  /** Arrow-key navigation: plain moves select the item, Shift extends the range. */
  const handleKeyboardSelect = useCallback(
    (path: string, extend: boolean) => {
      if (extend) {
        const anchor = anchorRef.current ?? selectedPaths[0] ?? null;
        if (anchorRef.current === null) anchorRef.current = anchor ?? path;
        setSelection(paneId, computeRangeSelection(files.map((f) => f.path), anchor, path));
      } else {
        anchorRef.current = path;
        setSelection(paneId, [path]);
      }
    },
    [paneId, selectedPaths, files, setSelection],
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
    handleKeyboardSelect,
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
