import { useMemo } from "react";
import { cn } from "@ufop/ui-components";
import { ChevronLeft, ChevronRight, Home } from "lucide-react";

/**
 * Breadcrumb bar with clickable path segments and back/forward navigation.
 * Shows the current directory path as breadcrumbs that can be clicked
 * to navigate to any parent directory.
 */
interface BreadcrumbBarProps {
  path: string;
  onNavigate: (path: string) => void;
  onBack?: () => void;
  onForward?: () => void;
  canGoBack?: boolean;
  canGoForward?: boolean;
  className?: string;
}

export function BreadcrumbBar({
  path,
  onNavigate,
  onBack,
  onForward,
  canGoBack = false,
  canGoForward = false,
  className,
}: BreadcrumbBarProps) {
  const segments = useMemo(() => {
    const parts = path.split("/").filter(Boolean);
    return parts.map((part, index) => ({
      label: part,
      path: "/" + parts.slice(0, index + 1).join("/"),
    }));
  }, [path]);

  return (
    <nav
      className={cn(
        "flex items-center gap-0.5 px-3 h-8 min-w-0",
        "bg-[var(--color-bg)] border-b border-[var(--color-border)]",
        "overflow-x-auto scrollbar-none",
        className,
      )}
      aria-label="Breadcrumb navigation"
      data-testid="breadcrumb-bar"
    >
      {/* Back / Forward buttons */}
      <button
        className={cn(
          "flex items-center justify-center w-6 h-6 rounded-[var(--radius-sm)]",
          "transition-theme shrink-0",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          canGoBack
            ? "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]"
            : "text-[color:var(--color-text-tertiary)] opacity-40 cursor-default",
        )}
        onClick={canGoBack ? onBack : undefined}
        disabled={!canGoBack}
        aria-label="Navigate back"
        data-testid="breadcrumb-back"
        type="button"
      >
        <ChevronLeft className="h-3.5 w-3.5" aria-hidden="true" />
      </button>
      <button
        className={cn(
          "flex items-center justify-center w-6 h-6 rounded-[var(--radius-sm)]",
          "transition-theme shrink-0",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          canGoForward
            ? "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]"
            : "text-[color:var(--color-text-tertiary)] opacity-40 cursor-default",
        )}
        onClick={canGoForward ? onForward : undefined}
        disabled={!canGoForward}
        aria-label="Navigate forward"
        data-testid="breadcrumb-forward"
        type="button"
      >
        <ChevronRight className="h-3.5 w-3.5" aria-hidden="true" />
      </button>

      {/* Separator */}
      <div className="w-px h-4 bg-[var(--color-border)] shrink-0 mx-0.5" />

      {/* Root / Home */}
      <button
        className={cn(
          "flex items-center gap-1 px-1.5 py-0.5 rounded-[var(--radius-sm)]",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
          "hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
          "transition-theme shrink-0",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={() => onNavigate("/")}
        aria-label="Navigate to root"
        data-testid="breadcrumb-root"
        type="button"
      >
        <Home className="h-3.5 w-3.5" aria-hidden="true" />
      </button>

      {segments.map((segment, index) => (
        <div key={segment.path} className="flex items-center gap-0.5 min-w-0">
          <ChevronRight
            className="h-3 w-3 text-[color:var(--color-text-tertiary)] shrink-0"
            aria-hidden="true"
          />
          <button
            className={cn(
              "px-1.5 py-0.5 rounded-[var(--radius-sm)]",
              "text-[length:var(--font-size-xs)] truncate max-w-[160px]",
              "transition-theme",
              "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
              index === segments.length - 1
                ? "text-[color:var(--color-text)] font-medium"
                : "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
            )}
            onClick={() => onNavigate(segment.path)}
            aria-current={index === segments.length - 1 ? "page" : undefined}
            title={segment.path}
            data-testid={`breadcrumb-segment-${index}`}
            type="button"
          >
            {segment.label}
          </button>
        </div>
      ))}
    </nav>
  );
}
