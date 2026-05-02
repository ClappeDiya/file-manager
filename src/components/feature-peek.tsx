import { useState, type ReactNode } from "react";
import { X, Sparkles } from "lucide-react";
import { useFeaturePeekStore } from "@/stores/feature-peek-store";

interface FeaturePeekProps {
  /** Stable id used to remember whether the user has dismissed this peek. */
  id: string;
  /** Short headline (e.g. "Tip: Activity ledger"). */
  title: string;
  /** One- or two-sentence body explaining what's behind the toggle. */
  children: ReactNode;
  /** Optional shortcut hint, rendered as a kbd. */
  shortcut?: string;
}

/**
 * Lightweight first-run hint card. Renders a one-time tip next to a feature
 * the user hasn't discovered yet. Persists dismissal across sessions via
 * `feature-peek-store`. No-op once dismissed.
 *
 * Usage:
 *   <FeaturePeek id="activity-panel" title="Watch live operations">
 *     The Activity panel streams every transfer, sync, and AI run as it happens.
 *   </FeaturePeek>
 */
export function FeaturePeek({ id, title, children, shortcut }: FeaturePeekProps) {
  const isDismissed = useFeaturePeekStore((s) => s.isDismissed(id));
  const dismiss = useFeaturePeekStore((s) => s.dismiss);
  const [closing, setClosing] = useState(false);

  if (isDismissed || closing) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      className="m-3 rounded-lg border border-[var(--color-primary)]/30 bg-[var(--color-primary)]/5 p-3 shadow-sm"
    >
      <div className="flex items-start gap-2">
        <Sparkles
          className="h-4 w-4 mt-0.5 shrink-0 text-[color:var(--color-primary)]"
          aria-hidden="true"
        />
        <div className="flex-1 min-w-0">
          <p className="text-xs font-semibold text-[color:var(--color-text)]">{title}</p>
          <p className="text-xs text-[color:var(--color-text-secondary)] mt-1 leading-relaxed">
            {children}
          </p>
          {shortcut && (
            <p className="text-[10px] text-[color:var(--color-text-tertiary)] mt-2">
              Shortcut:{" "}
              <kbd className="rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-1 py-0.5 font-mono">
                {shortcut}
              </kbd>
            </p>
          )}
        </div>
        <button
          type="button"
          onClick={() => {
            setClosing(true);
            dismiss(id);
          }}
          className="rounded p-0.5 text-[color:var(--color-text-tertiary)] hover:bg-[var(--color-hover-bg)] hover:text-[color:var(--color-text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
          aria-label={`Dismiss tip: ${title}`}
        >
          <X className="h-3 w-3" aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
