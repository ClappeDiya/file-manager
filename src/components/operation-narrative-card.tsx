/**
 * OperationNarrativeCard — inline plain-language narrative for a
 * correlation-id trace.
 *
 * Renders beneath the Activity Timeline's active correlation-trace chip
 * when the user clicks the "Explain" button. Reads nothing itself —
 * delegates to the backend's `narrator_narrate_operation` command via
 * {@link narrateOperation}, which does one SQL query against the
 * operation ledger and returns a deterministic story.
 *
 * # Design principles applied here
 *
 * - **Zero cognitive overload.** The card only renders when the user
 *   explicitly asks for a narrative. It is not a dashboard, not a
 *   notification, not a toast. Close it and it disappears.
 * - **Zero new infrastructure.** Composes the existing ledger query
 *   primitive and the deterministic `summarize()` function in Rust.
 *   No network, no AI dependency, no background work.
 * - **Always works.** The narrator falls back to a minimal story if
 *   the backend is unreachable, so the UI never has to handle null.
 * - **DRY visual grammar.** Uses the exact same CSS variables and
 *   lucide icons as the surrounding timeline panel. No new primitives.
 */
import { useEffect, useState } from "react";
import { BookOpen, Loader2, X } from "lucide-react";
import { narrateOperation, type OperationNarrative } from "@/lib/narrator";

interface OperationNarrativeCardProps {
  correlationId: string;
  onClose: () => void;
}

export function OperationNarrativeCard({
  correlationId,
  onClose,
}: OperationNarrativeCardProps) {
  const [narrative, setNarrative] = useState<OperationNarrative | null>(null);
  const [loading, setLoading] = useState(true);

  // Fetch on mount and whenever the correlation id changes. The card is
  // remounted with a fresh key every time the user clicks Explain, so
  // this effect is effectively single-shot per card.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    (async () => {
      const result = await narrateOperation(correlationId);
      if (!cancelled) {
        setNarrative(result);
        setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [correlationId]);

  return (
    <div
      className="border-b border-[var(--color-border)] bg-sky-500/5 px-3 py-2.5"
      role="region"
      aria-label="Operation narrative"
    >
      <div className="mb-1.5 flex items-start gap-2">
        <BookOpen className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-sky-500" aria-hidden="true" />
        <div className="flex-1 min-w-0">
          <p className="text-[11px] font-medium uppercase tracking-wider text-[color:var(--color-text-muted)]">
            Operation story
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close narrative"
          className="rounded p-0.5 text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
        >
          <X className="h-3 w-3" aria-hidden="true" />
        </button>
      </div>

      {loading && (
        <div className="flex items-center gap-2 text-xs text-[color:var(--color-text-muted)]">
          <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
          <span>Reading the ledger…</span>
        </div>
      )}

      {!loading && narrative && (
        <>
          <p className="text-xs font-medium text-[color:var(--color-text)]">
            {narrative.headline}
          </p>
          {narrative.story && narrative.story !== narrative.headline && (
            <p className="mt-1.5 text-xs leading-relaxed text-[color:var(--color-text-muted)]">
              {narrative.story}
            </p>
          )}
          {narrative.facts.length > 0 && (
            <ul className="mt-2 space-y-1">
              {narrative.facts.map((fact, idx) => (
                <li
                  key={`${fact.tag}-${idx}`}
                  className="flex items-start gap-1.5 text-[11px] text-[color:var(--color-text)]"
                >
                  <span
                    className="mt-1 inline-block h-1 w-1 flex-shrink-0 rounded-full bg-sky-500"
                    aria-hidden="true"
                  />
                  <span>{fact.text}</span>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </div>
  );
}
