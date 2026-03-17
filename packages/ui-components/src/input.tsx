import * as React from "react";
import { cn } from "./utils";

export interface InputProps
  extends React.InputHTMLAttributes<HTMLInputElement> {
  error?: boolean;
  /** Accessible label for screen readers when no visible label is present */
  srLabel?: string;
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type = "text", error, srLabel, id, ...props }, ref) => {
    const inputId = id || React.useId();

    return (
      <>
        {srLabel && (
          <label htmlFor={inputId} className="sr-only">
            {srLabel}
          </label>
        )}
        <input
          ref={ref}
          id={inputId}
          type={type}
          className={cn(
            // Base
            "flex h-9 w-full rounded-[var(--radius-md)] px-3 py-1",
            "text-[length:var(--font-size-base)] text-[color:var(--color-text)]",
            "bg-[var(--color-bg)] border border-[var(--color-border)]",
            "placeholder:text-[color:var(--color-text-tertiary)]",
            "transition-theme",
            // Focus
            "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
            "focus-visible:border-[var(--color-border-focus)]",
            // Hover
            "hover:border-[var(--color-border-hover)]",
            // Disabled
            "disabled:pointer-events-none disabled:opacity-50 disabled:bg-[var(--color-bg-secondary)]",
            // Error state
            error && "border-[var(--color-border-error)] focus-visible:outline-[var(--color-error)]",
            // File input
            "file:border-0 file:bg-transparent file:text-[length:var(--font-size-sm)] file:font-medium",
            className,
          )}
          aria-invalid={error || undefined}
          {...props}
        />
      </>
    );
  },
);

Input.displayName = "Input";
