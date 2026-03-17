import { useRef, useCallback } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
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
                  {item.size > 0
                    ? `${(item.size / 1024).toFixed(1)} KB`
                    : "--"}
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
