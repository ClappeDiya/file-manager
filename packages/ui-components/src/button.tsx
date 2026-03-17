import * as React from "react";
import { cn } from "./utils";

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "default"
    | "secondary"
    | "outline"
    | "ghost"
    | "destructive"
    | "link";
  size?: "sm" | "md" | "lg" | "icon";
  loading?: boolean;
}

const variantClasses: Record<NonNullable<ButtonProps["variant"]>, string> = {
  default:
    "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] hover:bg-[var(--color-primary-hover)] active:bg-[var(--color-primary-active)] shadow-[var(--shadow-xs)]",
  secondary:
    "bg-[var(--color-bg-secondary)] text-[color:var(--color-text)] hover:bg-[var(--color-bg-tertiary)] border border-[var(--color-border)]",
  outline:
    "border border-[var(--color-border)] bg-transparent text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)] hover:border-[var(--color-border-hover)]",
  ghost:
    "bg-transparent text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
  destructive:
    "bg-[var(--color-error)] text-white hover:bg-[var(--color-error-foreground)] shadow-[var(--shadow-xs)]",
  link: "bg-transparent text-[color:var(--color-primary)] hover:text-[color:var(--color-primary-hover)] underline-offset-4 hover:underline p-0 h-auto",
};

const sizeClasses: Record<NonNullable<ButtonProps["size"]>, string> = {
  sm: "h-8 px-3 text-[length:var(--font-size-sm)] rounded-[var(--radius-sm)]",
  md: "h-9 px-4 text-[length:var(--font-size-base)] rounded-[var(--radius-md)]",
  lg: "h-10 px-6 text-[length:var(--font-size-md)] rounded-[var(--radius-md)]",
  icon: "h-9 w-9 rounded-[var(--radius-md)]",
};

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      className,
      variant = "default",
      size = "md",
      loading = false,
      disabled,
      children,
      ...props
    },
    ref,
  ) => {
    return (
      <button
        ref={ref}
        className={cn(
          // Base
          "inline-flex items-center justify-center gap-2 font-medium whitespace-nowrap",
          "transition-theme",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
          "disabled:pointer-events-none disabled:opacity-50",
          "select-none cursor-pointer",
          // Variant
          variantClasses[variant],
          // Size
          sizeClasses[size],
          className,
        )}
        disabled={disabled || loading}
        aria-busy={loading || undefined}
        aria-disabled={disabled || loading || undefined}
        {...props}
      >
        {loading && (
          <svg
            className="h-4 w-4 animate-spin"
            xmlns="http://www.w3.org/2000/svg"
            fill="none"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
            />
          </svg>
        )}
        {children}
      </button>
    );
  },
);

Button.displayName = "Button";
