import type { HTMLAttributes } from "react";
import { useFocusTrap } from "@/hooks/use-focus-trap";

interface WorkspaceDialogProps extends HTMLAttributes<HTMLDivElement> {
  onClose: () => void;
  closeOnBackdrop?: boolean;
}

/** Shared keyboard and background isolation for file workspace overlays. */
export function WorkspaceDialog({
  onClose,
  closeOnBackdrop = true,
  children,
  ...props
}: WorkspaceDialogProps) {
  const ref = useFocusTrap<HTMLDivElement>({ active: true, onEscape: onClose });
  return (
    <div
      ref={ref}
      {...props}
      role="dialog"
      aria-modal="true"
      onClick={
        closeOnBackdrop
          ? (event) => {
              if (event.target === event.currentTarget) onClose();
            }
          : undefined
      }
    >
      {children}
    </div>
  );
}
