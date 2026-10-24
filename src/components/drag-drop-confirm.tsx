/**
 * Inline confirmation for drag-and-drop (Settings › General › "Ask before
 * drag-and-drop copies or moves"; off by default, so a drop acts at once as
 * it always has). Shown at the top of the target pane: what will be copied
 * or moved where, with the Safety Interlock's risk and reasons when the
 * interlock would ask (large or unusual moves). Confirming here also counts
 * as the interlock's confirmation, so the user is asked once.
 */
import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Copy, MoveRight } from "lucide-react";
import { cn } from "@ufop/ui-components";
import { assessIntent, type RiskAssessment } from "@/lib/safety";
import { formatBytes } from "@/lib/format-bytes";
import { dropIntent, type DropRequest } from "@/lib/drag-drop";

export function DragDropConfirm({
  request,
  onConfirm,
  onCancel,
}: {
  request: DropRequest;
  /** Called with the assessment shown (confirming waits for it). */
  onConfirm: (assessment: RiskAssessment | null) => void;
  onCancel: () => void;
}) {
  const [assessment, setAssessment] = useState<RiskAssessment | null>(null);
  const okRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    let live = true;
    setAssessment(null);
    void assessIntent(dropIntent(request)).then((a) => live && setAssessment(a));
    return () => {
      live = false;
    };
  }, [request]);
  // Confirm only once the risk is known (a quick Enter must not skip the
  // interlock); then the button takes the focus.
  const ready = assessment !== null;
  useEffect(() => {
    if (ready) okRef.current?.focus();
  }, [ready]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const n = request.sources.length;
  const risky = !!assessment?.requires_confirmation;
  const Icon = request.mode === "move" ? MoveRight : Copy;
  return (
    <div
      role="alertdialog"
      aria-label="Confirm drop"
      className={cn(
        "flex items-start gap-2 px-3 py-2 border-b text-[length:var(--font-size-xs)]",
        risky ? "bg-amber-500/10 border-amber-500/30" : "bg-[var(--color-bg-secondary)] border-[var(--color-border)]",
      )}
      data-testid="drag-drop-confirm"
    >
      {risky ? <AlertTriangle className="h-4 w-4 mt-0.5 text-amber-500 shrink-0" aria-hidden="true" /> : <Icon className="h-4 w-4 mt-0.5 shrink-0" aria-hidden="true" />}
      <div className="flex-1 min-w-0">
        <div className="font-medium text-[color:var(--color-text)]" data-testid="drag-drop-summary">
          {request.mode === "move" ? "Move" : "Copy"} {n} item{n === 1 ? "" : "s"} to {request.destLabel}?
          {request.totalBytes > 0 && <span className="font-normal text-[color:var(--color-text-secondary)]"> ({formatBytes(request.totalBytes)})</span>}
        </div>
        {risky && assessment && (
          <ul className="mt-1 list-disc pl-4 text-[11px] text-amber-700 dark:text-amber-300" data-testid="drag-drop-risk">
            <li className="list-none -ml-4 font-medium">{assessment.level === "high" ? "High risk" : "Unusual operation"}:</li>
            {assessment.reasons.map((r) => (
              <li key={r}>{r}</li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex items-center gap-1 shrink-0">
        <button
          type="button"
          onClick={onCancel}
          className="px-2 py-0.5 rounded border border-[var(--color-border)] hover:bg-[var(--color-hover-bg)]"
          data-testid="drag-drop-cancel"
        >
          Cancel
        </button>
        <button
          ref={okRef}
          type="button"
          disabled={!ready}
          aria-busy={!ready}
          title={ready ? undefined : "Checking the operation with the Safety Interlock…"}
          onClick={() => ready && onConfirm(assessment)}
          className={cn("px-2 py-0.5 rounded text-white disabled:opacity-50", risky ? "bg-amber-600 hover:bg-amber-700" : "bg-[var(--color-primary)] hover:opacity-90")}
          data-testid="drag-drop-ok"
        >
          {request.mode === "move" ? "Move" : "Copy"}
        </button>
      </div>
    </div>
  );
}
