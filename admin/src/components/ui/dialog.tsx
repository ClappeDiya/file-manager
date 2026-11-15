"use client";

import React from "react";
import { useFocusTrap } from "@ufop/ui-components";
import { cn } from "@/lib/utils/cn";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  children: React.ReactNode;
  className?: string;
}

const DialogTitleId = React.createContext<string | undefined>(undefined);

export function Dialog({ open, onClose, children, className }: DialogProps) {
  const titleId = React.useId();
  const panelRef = useFocusTrap<HTMLDivElement>({ active: open, onEscape: onClose });

  if (!open) return null;

  return (
    <DialogTitleId.Provider value={titleId}>
      <div className="fixed inset-0 z-50 flex items-center justify-center">
        <div
          className="fixed inset-0 bg-black/50"
          onClick={onClose}
          aria-hidden="true"
        />
        <div
          ref={panelRef}
          className={cn(
            "relative z-50 w-full max-w-lg bg-background-elevated rounded-xl border border-border shadow-xl p-6 mx-4 max-h-[90dvh] overflow-y-auto",
            "animate-in fade-in-0 zoom-in-95",
            className,
          )}
          aria-labelledby={titleId}
          role="dialog"
          aria-modal="true"
        >
          {children}
        </div>
      </div>
    </DialogTitleId.Provider>
  );
}

export function DialogHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("mb-4", className)} {...props} />;
}

export function DialogTitle({
  className,
  ...props
}: React.HTMLAttributes<HTMLHeadingElement>) {
  const titleId = React.useContext(DialogTitleId);
  return (
    <h2
      id={titleId}
      className={cn("text-lg font-semibold text-foreground", className)}
      {...props}
    />
  );
}

export function DialogDescription({
  className,
  ...props
}: React.HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p
      className={cn("text-sm text-foreground-secondary mt-1", className)}
      {...props}
    />
  );
}

export function DialogFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("sticky bottom-0 z-10 flex flex-wrap items-center justify-end gap-2 mt-6 border-t border-border bg-background-elevated pt-3", className)}
      {...props}
    />
  );
}
