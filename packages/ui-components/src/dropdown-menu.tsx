'use client';

import * as React from "react";
import { cn } from "./utils";

/* ===== DropdownMenu Context ===== */
interface DropdownMenuContextValue {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  triggerRef: React.RefObject<HTMLButtonElement | null>;
}

const DropdownMenuContext = React.createContext<DropdownMenuContextValue | null>(
  null,
);

function useDropdownMenuContext() {
  const ctx = React.useContext(DropdownMenuContext);
  if (!ctx)
    throw new Error("DropdownMenu components must be used within DropdownMenu");
  return ctx;
}

/* ===== Root ===== */
export interface DropdownMenuProps {
  children: React.ReactNode;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export function DropdownMenu({
  children,
  open: controlledOpen,
  onOpenChange: controlledOnOpenChange,
}: DropdownMenuProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(false);
  const triggerRef = React.useRef<HTMLButtonElement>(null);

  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : uncontrolledOpen;
  const onOpenChange = isControlled
    ? controlledOnOpenChange!
    : setUncontrolledOpen;

  return (
    <DropdownMenuContext.Provider value={{ open, onOpenChange, triggerRef }}>
      <div className="relative inline-block">{children}</div>
    </DropdownMenuContext.Provider>
  );
}

/* ===== Trigger ===== */
export function DropdownMenuTrigger({
  children,
  className,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  const { open, onOpenChange, triggerRef } = useDropdownMenuContext();

  return (
    <button
      ref={triggerRef}
      className={className}
      onClick={() => onOpenChange(!open)}
      aria-expanded={open}
      aria-haspopup="menu"
      type="button"
      {...props}
    >
      {children}
    </button>
  );
}

/* ===== Content ===== */
export interface DropdownMenuContentProps
  extends React.HTMLAttributes<HTMLDivElement> {
  align?: "start" | "center" | "end";
  side?: "top" | "bottom";
}

export function DropdownMenuContent({
  className,
  align = "start",
  side = "bottom",
  children,
  ...props
}: DropdownMenuContentProps) {
  const { open, onOpenChange } = useDropdownMenuContext();
  const menuRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (!open) return;

    // Focus first menu item
    const items = menuRef.current?.querySelectorAll<HTMLElement>(
      '[role="menuitem"]:not([disabled])',
    );
    items?.[0]?.focus();

    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onOpenChange(false);
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onOpenChange(false);
        return;
      }

      if (!items || items.length === 0) return;

      const currentIndex = Array.from(items).findIndex(
        (item) => item === document.activeElement,
      );

      if (e.key === "ArrowDown") {
        e.preventDefault();
        const nextIndex = currentIndex < items.length - 1 ? currentIndex + 1 : 0;
        items[nextIndex].focus();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        const prevIndex = currentIndex > 0 ? currentIndex - 1 : items.length - 1;
        items[prevIndex].focus();
      } else if (e.key === "Home") {
        e.preventDefault();
        items[0].focus();
      } else if (e.key === "End") {
        e.preventDefault();
        items[items.length - 1].focus();
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onOpenChange]);

  if (!open) return null;

  const alignClasses = {
    start: "left-0",
    center: "left-1/2 -translate-x-1/2",
    end: "right-0",
  };

  const sideClasses = {
    top: "bottom-full mb-1",
    bottom: "top-full mt-1",
  };

  return (
    <div
      ref={menuRef}
      role="menu"
      aria-orientation="vertical"
      className={cn(
        "absolute z-50 min-w-[8rem] overflow-hidden",
        "rounded-[var(--radius-md)] border border-[var(--color-border)]",
        "bg-[var(--color-bg-elevated)] shadow-[var(--shadow-md)]",
        "p-1 animate-slide-in",
        alignClasses[align],
        sideClasses[side],
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}

/* ===== Item ===== */
export interface DropdownMenuItemProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  destructive?: boolean;
}

export function DropdownMenuItem({
  className,
  destructive,
  disabled,
  children,
  onClick,
  ...props
}: DropdownMenuItemProps) {
  const { onOpenChange } = useDropdownMenuContext();

  return (
    <button
      role="menuitem"
      className={cn(
        "flex w-full items-center gap-2 rounded-[var(--radius-sm)] px-2 py-1.5",
        "text-[length:var(--font-size-sm)] text-left",
        "transition-theme cursor-pointer",
        "focus-visible:outline-none focus:bg-[var(--color-hover-bg)]",
        "hover:bg-[var(--color-hover-bg)]",
        destructive
          ? "text-[color:var(--color-error)] focus:text-[color:var(--color-error)]"
          : "text-[color:var(--color-text)]",
        disabled && "pointer-events-none opacity-50",
        className,
      )}
      disabled={disabled}
      tabIndex={-1}
      onClick={(e) => {
        onClick?.(e);
        onOpenChange(false);
      }}
      type="button"
      {...props}
    >
      {children}
    </button>
  );
}

/* ===== Separator ===== */
export function DropdownMenuSeparator({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      role="separator"
      className={cn("my-1 h-px bg-[var(--color-border)]", className)}
      {...props}
    />
  );
}

/* ===== Label ===== */
export function DropdownMenuLabel({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "px-2 py-1.5 text-[length:var(--font-size-xs)] font-semibold text-[color:var(--color-text-secondary)]",
        className,
      )}
      {...props}
    />
  );
}
