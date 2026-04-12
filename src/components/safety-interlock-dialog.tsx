/**
 * SafetyInterlockDialog — global confirmation surface for risky operations.
 *
 * The fifth feature in the ledger-axis product line (after Activity
 * Timeline, Universal Undo, File Lineage, and Instant Jump). Where the
 * prior features ask retrospective or navigational questions about the
 * ledger, this one uses the ledger *prospectively*: before any risky
 * operation runs, the backend compares the intent to the user's own
 * history and decides whether to pause.
 *
 * # Design principles applied here
 *
 * - **Zero cognitive overload**: the dialog only appears when an
 *   operation is statistically outside the user's own normal pattern.
 *   Low-risk operations never trigger it, so the user sees it perhaps
 *   once a month instead of on every file move.
 * - **Per-user baseline**: "normal" is defined by the user's own prior
 *   operations of the same shape, not by an arbitrary global threshold,
 *   so power users aren't nagged and new users are protected.
 * - **Fail-open**: if the interlock itself errors, the backend returns a
 *   `low` assessment and this dialog never appears. A broken safety
 *   layer can never block legitimate work.
 * - **Single wrap-point**: callers never import this component. They
 *   call `assessBeforeExecute()` from `src/lib/safety.ts`, which
 *   toggles this dialog via the safety store. DRY by construction.
 * - **Auditable**: the user's decision is written back to the operation
 *   ledger as a `safety.confirmed` event, so a future review can trace
 *   every override.
 */
import { useCallback, useEffect, useMemo, useRef, type KeyboardEvent } from "react";
import { formatBytes } from "@/lib/format-bytes";
import { useSafetyStore } from "@/stores/safety-store";
import { AlertTriangle, ShieldAlert, X } from "lucide-react";

export function SafetyInterlockDialog() {
  const pending = useSafetyStore((s) => s.pending);
  const approve = useSafetyStore((s) => s.approve);
  const reject = useSafetyStore((s) => s.reject);

  const cancelButtonRef = useRef<HTMLButtonElement | null>(null);
  const proceedButtonRef = useRef<HTMLButtonElement | null>(null);

  // High-risk defaults focus to Cancel (destructive-safe default);
  // medium-risk defaults focus to Proceed (flows that just need a nudge).
  const isHigh = pending?.assessment.level === "high";

  useEffect(() => {
    if (!pending) return;
    const target = isHigh ? cancelButtonRef.current : proceedButtonRef.current;
    target?.focus();
  }, [pending, isHigh]);

  // Keyboard: Escape always rejects. Enter approves only if the current
  // focus isn't already on the cancel button (which would be double-action).
  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLDivElement>) => {
      if (e.key === "Escape") {
        e.preventDefault();
        reject();
      }
    },
    [reject],
  );

  const displaySummary = useMemo(() => {
    if (!pending) return null;
    const i = pending.intent;
    const parts: string[] = [];
    if (i.affected_files > 0) {
      parts.push(`${i.affected_files.toLocaleString()} file${i.affected_files === 1 ? "" : "s"}`);
    }
    if (i.total_bytes > 0) {
      parts.push(formatBytes(i.total_bytes, { binary: true }));
    }
    if (parts.length === 0) parts.push("1 item");
    return `${i.engine} · ${i.kind} · ${parts.join(" · ")}`;
  }, [pending]);

  if (!pending) return null;

  const { assessment, intent } = pending;
  const iconColor = isHigh ? "text-red-400" : "text-amber-300";
  const accentBorder = isHigh ? "border-red-500/50" : "border-amber-500/40";
  const badge = isHigh ? "High risk" : "Unusual operation";

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="safety-interlock-title"
      aria-describedby="safety-interlock-reasons"
      onKeyDown={onKeyDown}
    >
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        aria-hidden="true"
      />

      <div
        className={`relative z-10 w-full max-w-lg rounded-lg border ${accentBorder} bg-[var(--color-bg-secondary)] shadow-2xl`}
      >
        <div className="flex items-start gap-3 border-b border-[var(--color-border)] px-4 py-3">
          {isHigh ? (
            <ShieldAlert className={`h-5 w-5 flex-shrink-0 ${iconColor}`} aria-hidden="true" />
          ) : (
            <AlertTriangle className={`h-5 w-5 flex-shrink-0 ${iconColor}`} aria-hidden="true" />
          )}
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <h2
                id="safety-interlock-title"
                className="text-sm font-semibold text-[color:var(--color-text)]"
              >
                Confirm before proceeding
              </h2>
              <span
                className={`rounded-full border ${accentBorder} px-2 py-0.5 text-[10px] uppercase tracking-wider ${iconColor}`}
              >
                {badge}
              </span>
            </div>
            {displaySummary && (
              <p className="mt-1 truncate text-xs text-[color:var(--color-text-muted)]">
                {displaySummary}
              </p>
            )}
          </div>
          <button
            type="button"
            aria-label="Cancel"
            onClick={reject}
            className="rounded p-1 text-[color:var(--color-text-muted)] hover:bg-[var(--color-bg-tertiary)] hover:text-[color:var(--color-text)]"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <div className="px-4 py-3">
          {intent.summary && (
            <p className="mb-2 text-sm text-[color:var(--color-text)]">
              {intent.summary}
            </p>
          )}
          <p className="mb-2 text-xs font-medium uppercase tracking-wider text-[color:var(--color-text-muted)]">
            Why the interlock paused this
          </p>
          <ul
            id="safety-interlock-reasons"
            className="space-y-1.5 text-sm text-[color:var(--color-text)]"
          >
            {assessment.reasons.map((reason, idx) => (
              <li key={idx} className="flex items-start gap-2">
                <span className={`mt-1 inline-block h-1.5 w-1.5 flex-shrink-0 rounded-full ${iconColor}`} />
                <span>{reason}</span>
              </li>
            ))}
          </ul>

          {assessment.baseline_sample_size > 0 && (
            <p className="mt-3 text-[11px] text-[color:var(--color-text-muted)]">
              Baseline: {assessment.baseline_sample_size} prior {intent.kind} run
              {assessment.baseline_sample_size === 1 ? "" : "s"} · median{" "}
              {assessment.baseline_median_files} file
              {assessment.baseline_median_files === 1 ? "" : "s"}
              {assessment.baseline_median_bytes > 0
                ? ` · ${formatBytes(assessment.baseline_median_bytes, { binary: true })}`
                : ""}
            </p>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <button
            ref={cancelButtonRef}
            type="button"
            onClick={reject}
            className="rounded border border-[var(--color-border)] bg-transparent px-3 py-1.5 text-sm text-[color:var(--color-text)] hover:bg-[var(--color-bg-tertiary)] focus:outline-none focus:ring-2 focus:ring-sky-500/50"
          >
            Cancel
          </button>
          <button
            ref={proceedButtonRef}
            type="button"
            onClick={approve}
            className={`rounded px-3 py-1.5 text-sm font-medium text-white focus:outline-none focus:ring-2 ${
              isHigh
                ? "bg-red-600 hover:bg-red-500 focus:ring-red-400/60"
                : "bg-amber-600 hover:bg-amber-500 focus:ring-amber-400/60"
            }`}
          >
            {isHigh ? "Proceed anyway" : "Proceed"}
          </button>
        </div>
      </div>
    </div>
  );
}
