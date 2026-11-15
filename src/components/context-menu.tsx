import { useUIStore } from "@/stores/ui-store";
import { cn } from "@ufop/ui-components";
import { useCallback,useEffect,useRef,useState } from "react";
import { ContextMenuItem } from './context-menu-helpers';

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
    adjustedPosition.y = Math.max(8, adjustedPosition.y);
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
        maxHeight: `calc(100vh - ${adjustedPosition.y + 8}px)`,
        overflowY: "auto",
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
            title={item.disabled ? item.disabledReason : undefined}
            aria-description={item.disabled ? item.disabledReason : undefined}
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
