'use client';

import * as React from "react";
import { cn } from "./utils";

/* ===== Dialog Context ===== */
interface DialogContextValue {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const DialogContext = React.createContext<DialogContextValue | null>(null);

function useDialogContext() {
  const context = React.useContext(DialogContext);
  if (!context) {
    throw new Error("Dialog components must be used within a Dialog provider");
  }
  return context;
}

/* ===== Dialog Root ===== */
export interface DialogProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  defaultOpen?: boolean;
  children: React.ReactNode;
}

export function Dialog({
  open: controlledOpen,
  onOpenChange: controlledOnOpenChange,
  defaultOpen = false,
  children,
}: DialogProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);

  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : uncontrolledOpen;
  const onOpenChange = isControlled
    ? controlledOnOpenChange!
    : setUncontrolledOpen;

  return (
    <DialogContext.Provider value={{ open, onOpenChange }}>
      {children}
    </DialogContext.Provider>
  );
}

/* ===== Dialog Trigger ===== */
export interface DialogTriggerProps {
  children: React.ReactElement<React.HTMLAttributes<HTMLElement>>;
  asChild?: boolean;
}

export function DialogTrigger({ children }: DialogTriggerProps) {
  const { onOpenChange } = useDialogContext();

  return React.cloneElement(children, {
    onClick: (e: React.MouseEvent<HTMLElement>) => {
      children.props.onClick?.(e);
      onOpenChange(true);
    },
  } as React.HTMLAttributes<HTMLElement>);
}

/* ===== Dialog Overlay ===== */
export function DialogOverlay({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { open, onOpenChange } = useDialogContext();

  if (!open) return null;

  return (
    <div
      className={cn(
        "fixed inset-0 z-50 bg-[var(--color-bg-overlay)]",
        "animate-fade-in",
        className,
      )}
      onClick={() => onOpenChange(false)}
      aria-hidden="true"
      {...props}
    />
  );
}

/* ===== Dialog Content ===== */
export interface DialogContentProps
  extends React.HTMLAttributes<HTMLDivElement> {
  /** Width of the dialog */
  size?: "sm" | "md" | "lg" | "xl";
}

const sizeClasses = {
  sm: "max-w-sm",
  md: "max-w-md",
  lg: "max-w-lg",
  xl: "max-w-xl",
};

export function DialogContent({
  className,
  size = "md",
  children,
  ...props
}: DialogContentProps) {
  const { open, onOpenChange } = useDialogContext();
  const contentRef = React.useRef<HTMLDivElement>(null);

  // Trap focus inside dialog
  React.useEffect(() => {
    if (!open) return;

    const content = contentRef.current;
    if (!content) return;

    // Focus the first focusable element
    const focusable = content.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    if (focusable.length > 0) {
      focusable[0].focus();
    }

    // Handle Escape key
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onOpenChange(false);
      }
      // Trap focus
      if (e.key === "Tab") {
        const firstFocusable = focusable[0];
        const lastFocusable = focusable[focusable.length - 1];
        if (e.shiftKey) {
          if (document.activeElement === firstFocusable) {
            e.preventDefault();
            lastFocusable?.focus();
          }
        } else {
          if (document.activeElement === lastFocusable) {
            e.preventDefault();
            firstFocusable?.focus();
          }
        }
      }
    };

    document.addEventListener("keydown", handleKeyDown);

    // Prevent body scroll
    const originalOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = originalOverflow;
    };
  }, [open, onOpenChange]);

  if (!open) return null;

  return (
    <>
      <DialogOverlay />
      <div
        ref={contentRef}
        role="dialog"
        aria-modal="true"
        className={cn(
          "fixed inset-0 z-50 flex items-center justify-center p-4",
        )}
      >
        <div
          className={cn(
            "w-full bg-[var(--color-bg-elevated)] rounded-[var(--radius-lg)]",
            "shadow-[var(--shadow-lg)] border border-[var(--color-border)]",
            "animate-scale-in",
            "max-h-[85vh] overflow-y-auto",
            sizeClasses[size],
            className,
          )}
          onClick={(e) => e.stopPropagation()}
          {...props}
        >
          {children}
        </div>
      </div>
    </>
  );
}

/* ===== Dialog Header ===== */
export function DialogHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 p-6 pb-0",
        className,
      )}
      {...props}
    />
  );
}

/* ===== Dialog Title ===== */
export function DialogTitle({
  className,
  ...props
}: React.HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h2
      className={cn(
        "text-[length:var(--font-size-xl)] font-semibold text-[color:var(--color-text)] leading-tight",
        className,
      )}
      {...props}
    />
  );
}

/* ===== Dialog Description ===== */
export function DialogDescription({
  className,
  ...props
}: React.HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p
      className={cn(
        "text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]",
        className,
      )}
      {...props}
    />
  );
}

/* ===== Dialog Footer ===== */
export function DialogFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "flex justify-end gap-2 p-6 pt-4",
        className,
      )}
      {...props}
    />
  );
}

/* ===== Dialog Close ===== */
export function DialogClose({
  children,
}: {
  children: React.ReactElement<React.HTMLAttributes<HTMLElement>>;
}) {
  const { onOpenChange } = useDialogContext();

  return React.cloneElement(children, {
    onClick: (e: React.MouseEvent<HTMLElement>) => {
      children.props.onClick?.(e);
      onOpenChange(false);
    },
  } as React.HTMLAttributes<HTMLElement>);
}
