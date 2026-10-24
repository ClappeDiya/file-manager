import { useRef, useCallback, useEffect, useState, type ReactNode } from "react";
import { formatBytes } from "@/lib/format-bytes";
import { observeElementRect, useVirtualizer } from "@tanstack/react-virtual";
import { cn } from "@ufop/ui-components";
import type { FileEntry } from "./data-grid";

interface VirtualListProps {
  items: FileEntry[];
  className?: string;
  rowHeight?: number;
}

/**
 * Virtualized file list using TanStack Virtual.
 * Efficiently renders 10K+ items by only rendering visible rows.
 * Includes keyboard navigation and screen reader support.
 */
export function VirtualList({
  items,
  className,
  rowHeight = 36,
}: VirtualListProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const activeIndexRef = useRef(0);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 10,
  });

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const currentIndex = activeIndexRef.current;

      switch (e.key) {
        case "ArrowDown": {
          e.preventDefault();
          const nextIndex = Math.min(currentIndex + 1, items.length - 1);
          activeIndexRef.current = nextIndex;
          virtualizer.scrollToIndex(nextIndex, { align: "auto" });
          // Force re-render by focusing the container
          parentRef.current?.focus();
          break;
        }
        case "ArrowUp": {
          e.preventDefault();
          const prevIndex = Math.max(currentIndex - 1, 0);
          activeIndexRef.current = prevIndex;
          virtualizer.scrollToIndex(prevIndex, { align: "auto" });
          parentRef.current?.focus();
          break;
        }
        case "Home": {
          e.preventDefault();
          activeIndexRef.current = 0;
          virtualizer.scrollToIndex(0);
          parentRef.current?.focus();
          break;
        }
        case "End": {
          e.preventDefault();
          activeIndexRef.current = items.length - 1;
          virtualizer.scrollToIndex(items.length - 1);
          parentRef.current?.focus();
          break;
        }
        case "PageDown": {
          e.preventDefault();
          const pageSize = Math.floor(
            (parentRef.current?.clientHeight || 400) / rowHeight,
          );
          const newIndex = Math.min(currentIndex + pageSize, items.length - 1);
          activeIndexRef.current = newIndex;
          virtualizer.scrollToIndex(newIndex, { align: "auto" });
          parentRef.current?.focus();
          break;
        }
        case "PageUp": {
          e.preventDefault();
          const pageSizeUp = Math.floor(
            (parentRef.current?.clientHeight || 400) / rowHeight,
          );
          const newIdx = Math.max(currentIndex - pageSizeUp, 0);
          activeIndexRef.current = newIdx;
          virtualizer.scrollToIndex(newIdx, { align: "auto" });
          parentRef.current?.focus();
          break;
        }
      }
    },
    [items.length, rowHeight, virtualizer],
  );

  return (
    <div className={cn("space-y-2", className)}>
      <div className="flex items-center justify-between">
        <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]">
          Virtualized List ({items.length.toLocaleString()} items)
        </span>
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]" aria-live="polite">
          Rendering {virtualizer.getVirtualItems().length} visible rows
        </span>
      </div>

      <div
        ref={parentRef}
        className={cn(
          "overflow-auto rounded-[var(--radius-md)] border border-[var(--color-border)]",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        style={{ height: "400px" }}
        role="listbox"
        aria-label="Virtual file list"
        aria-activedescendant={`vlist-item-${activeIndexRef.current}`}
        tabIndex={0}
        onKeyDown={handleKeyDown}
      >
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((virtualItem) => {
            const item = items[virtualItem.index];
            const isActive = virtualItem.index === activeIndexRef.current;

            return (
              <div
                key={virtualItem.key}
                id={`vlist-item-${virtualItem.index}`}
                role="option"
                aria-selected={isActive}
                className={cn(
                  "absolute left-0 top-0 w-full flex items-center px-3 gap-3",
                  "border-b border-[var(--color-border)]",
                  "transition-theme",
                  isActive
                    ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
                    : "hover:bg-[var(--color-hover-bg)]",
                )}
                style={{
                  height: `${virtualItem.size}px`,
                  transform: `translateY(${virtualItem.start}px)`,
                }}
                onClick={() => {
                  activeIndexRef.current = virtualItem.index;
                  parentRef.current?.focus();
                }}
              >
                <span
                  className={cn(
                    "text-[length:var(--font-size-sm)] truncate flex-1",
                    item.type === "folder"
                      ? "font-medium text-[color:var(--color-primary)]"
                      : "text-[color:var(--color-text)]",
                  )}
                >
                  {item.name}
                </span>
                <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] w-20 text-right">
                  {item.size > 0 ? formatBytes(item.size) : "--"}
                </span>
                <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] w-24 text-right">
                  {new Date(item.modified).toLocaleDateString()}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// ── Generic virtualised rows ──

export interface VirtualRowsProps<T> {
  items: T[];
  /** Draws one row's content; the row box (height, selection colour) is ours. */
  renderRow: (item: T, index: number) => ReactNode;
  ariaLabel: string;
  rowHeight?: number;
  /** Height of the scroll area in px. */
  height?: number;
  /** Rows can be selected (click, Ctrl/Cmd-click, Shift-click, Shift+arrows, Ctrl/Cmd+A). */
  selectable?: boolean;
  /** Told the selected row indexes, ascending, whenever the selection changes. */
  onSelectionChange?: (indexes: number[]) => void;
  /** Ctrl/Cmd+C on the list: told the selected row indexes. */
  onCopy?: (indexes: number[]) => void;
  /** Stay on the newest (last) row while rows are added, until the reader scrolls up. */
  followTail?: boolean;
  /** Row `id` attribute; defaults to the index. */
  rowKey?: (item: T, index: number) => string | number;
  className?: string;
}

/**
 * A scrolling list that only renders the rows in view (TanStack Virtual), so
 * tens of thousands of rows stay quick: the log viewer and the changed-items
 * list of a sync run. Keyboard and screen-reader behaviour follows
 * [`VirtualList`].
 */
export function VirtualRows<T>({
  items,
  renderRow,
  ariaLabel,
  rowHeight = 22,
  height = 360,
  selectable = false,
  onSelectionChange,
  onCopy,
  followTail = false,
  rowKey,
  className,
}: VirtualRowsProps<T>) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [active, setActive] = useState(0);
  const anchor = useRef(0);
  const atTail = useRef(true);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    initialRect: { width: 800, height },
    // An element that reports no size (hidden, or a test DOM) still gets its rows drawn.
    observeElementRect: (instance, cb) => observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : { width: rect.width || 800, height })),
  });

  const commit = useCallback(
    (next: Set<number>) => {
      setSelected(next);
      onSelectionChange?.([...next].sort((a, b) => a - b));
    },
    [onSelectionChange],
  );

  // A new list (filters changed, rows cleared) starts without a selection.
  const length = items.length;
  useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev;
      const kept = new Set([...prev].filter((i) => i < length));
      if (kept.size === prev.size) return prev;
      onSelectionChange?.([...kept].sort((a, b) => a - b));
      return kept;
    });
    setActive((a) => Math.min(a, Math.max(0, length - 1)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [length]);

  // Follow the tail while the reader has not scrolled away from it.
  useEffect(() => {
    if (followTail && atTail.current && length > 0) virtualizer.scrollToIndex(length - 1, { align: "end" });
  }, [followTail, length, virtualizer]);

  const onScroll = useCallback(() => {
    const el = parentRef.current;
    if (el) atTail.current = el.scrollTop + el.clientHeight >= el.scrollHeight - rowHeight * 2;
  }, [rowHeight]);

  const pick = useCallback(
    (index: number, mods: { shift: boolean; toggle: boolean }) => {
      if (!selectable) return;
      if (mods.shift) {
        const [a, b] = [Math.min(anchor.current, index), Math.max(anchor.current, index)];
        const range = new Set<number>(mods.toggle ? selected : []);
        for (let i = a; i <= b; i++) range.add(i);
        commit(range);
      } else if (mods.toggle) {
        const next = new Set(selected);
        if (!next.delete(index)) next.add(index);
        anchor.current = index;
        commit(next);
      } else {
        anchor.current = index;
        commit(new Set([index]));
      }
      setActive(index);
    },
    [selectable, selected, commit],
  );

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (length === 0) return;
    const mod = e.ctrlKey || e.metaKey;
    const move = (to: number) => {
      const i = Math.max(0, Math.min(length - 1, to));
      e.preventDefault();
      virtualizer.scrollToIndex(i, { align: "auto" });
      if (selectable) pick(i, { shift: e.shiftKey, toggle: false });
      else setActive(i);
    };
    const page = Math.max(1, Math.floor((parentRef.current?.clientHeight || height) / rowHeight));
    switch (e.key) {
      case "ArrowDown": return move(active + 1);
      case "ArrowUp": return move(active - 1);
      case "PageDown": return move(active + page);
      case "PageUp": return move(active - page);
      case "Home": return move(0);
      case "End": return move(length - 1);
      case "a":
      case "A":
        if (mod && selectable) {
          e.preventDefault();
          commit(new Set(Array.from({ length }, (_, i) => i)));
        }
        return;
      case "c":
      case "C":
        if (mod && onCopy && selected.size > 0) {
          e.preventDefault();
          onCopy([...selected].sort((a, b) => a - b));
        }
        return;
    }
  };

  return (
    <div
      ref={parentRef}
      className={cn(
        "overflow-auto rounded-[var(--radius-md)] border border-[var(--color-border)]",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
        className,
      )}
      style={{ height }}
      role="listbox"
      aria-label={ariaLabel}
      aria-multiselectable={selectable || undefined}
      aria-rowcount={length}
      aria-activedescendant={length > 0 ? `vrow-${ariaLabel}-${active}` : undefined}
      tabIndex={0}
      onKeyDown={onKeyDown}
      onScroll={onScroll}
    >
      <div style={{ height: `${virtualizer.getTotalSize()}px`, width: "100%", position: "relative" }}>
        {virtualizer.getVirtualItems().map((v) => {
          const item = items[v.index];
          if (item === undefined) return null;
          const isSel = selected.has(v.index);
          return (
            <div
              key={rowKey ? rowKey(item, v.index) : v.key}
              id={`vrow-${ariaLabel}-${v.index}`}
              role="option"
              aria-selected={selectable ? isSel : undefined}
              aria-posinset={v.index + 1}
              aria-setsize={length}
              className={cn(
                "absolute left-0 top-0 w-full flex items-center px-2 gap-2",
                isSel ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]" : "hover:bg-[var(--color-hover-bg)]",
                v.index === active && "outline outline-1 -outline-offset-1 outline-[var(--color-focus-ring)]",
              )}
              style={{ height: `${v.size}px`, transform: `translateY(${v.start}px)` }}
              onClick={(e) => pick(v.index, { shift: e.shiftKey, toggle: e.ctrlKey || e.metaKey })}
            >
              {renderRow(item, v.index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
