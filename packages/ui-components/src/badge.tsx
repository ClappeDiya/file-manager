'use client';

import * as React from "react";
import { cn } from "./utils";

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: "default" | "secondary" | "success" | "warning" | "error" | "info" | "outline";
}

const variantClasses: Record<NonNullable<BadgeProps["variant"]>, string> = {
  default:
    "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]",
  secondary:
    "bg-[var(--color-bg-tertiary)] text-[color:var(--color-text-secondary)]",
  success:
    "bg-[var(--color-success-bg)] text-[color:var(--color-success-foreground)]",
  warning:
    "bg-[var(--color-warning-bg)] text-[color:var(--color-warning-foreground)]",
  error:
    "bg-[var(--color-error-bg)] text-[color:var(--color-error-foreground)]",
  info:
    "bg-[var(--color-info-bg)] text-[color:var(--color-info-foreground)]",
  outline:
    "bg-transparent text-[color:var(--color-text)] border border-[var(--color-border)]",
};

export const Badge = React.forwardRef<HTMLSpanElement, BadgeProps>(
  ({ className, variant = "default", ...props }, ref) => {
    return (
      <span
        ref={ref}
        className={cn(
          "inline-flex items-center rounded-[var(--radius-full)] px-2 py-0.5",
          "text-[length:var(--font-size-xs)] font-medium leading-none",
          "select-none whitespace-nowrap",
          variantClasses[variant],
          className,
        )}
        {...props}
      />
    );
  },
);

Badge.displayName = "Badge";
