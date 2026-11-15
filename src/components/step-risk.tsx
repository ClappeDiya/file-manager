import { HighRiskStep } from './step-risk-helpers';
/**
 * Per-step risk in the rule editor and workflow builder (Advanced). The
 * classification itself lives in Rust (`automation_engine::risk`, one place
 * for the badge, the editor's confirmation and the tests); this file only
 * asks for it and draws it. Saving a rule that contains a High step needs an
 * explicit answer in an inline dialog.
 */
import type { RiskLevel,StepRisk } from "@/stores/automation-store";

const TONE: Record<RiskLevel, string> = {
  low: "bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300",
  medium: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
  high: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
};
const WORD: Record<RiskLevel, string> = { low: "Low risk", medium: "Medium risk", high: "High risk" };

export function RiskBadge({ risk }: { risk: StepRisk | undefined }) {
  if (!risk) return null;
  return (
    <span className={`inline-block rounded px-1.5 py-0.5 text-[10px] font-medium ${TONE[risk.level]}`} title={risk.reason} data-testid="risk-badge" data-risk={risk.level}>
      {WORD[risk.level]}
    </span>
  );
}

/** Inline confirmation shown before saving a rule with High-risk steps (not `window.confirm`). */
export function RiskConfirm({ steps, onConfirm, onCancel }: { steps: HighRiskStep[]; onConfirm: () => void; onCancel: () => void }) {
  return (
    <section role="alertdialog" aria-labelledby="risk-confirm-title" className="rounded border border-red-500/60 bg-red-500/5 p-3 text-xs space-y-2" data-testid="risk-confirm">
      <p id="risk-confirm-title" className="font-semibold text-red-700 dark:text-red-300">
        This rule has {steps.length === 1 ? "a high-risk step" : `${steps.length} high-risk steps`}. Save it anyway?
      </p>
      <ul className="list-disc pl-5">
        {steps.map((s) => (
          <li key={s.step}>
            {s.step}: {s.reason}
          </li>
        ))}
      </ul>
      <p className="text-[11px] text-[var(--color-text-muted)]">Check the folders and conditions with a dry run first. Steps that remove data are checked again by the safety interlock when they run.</p>
      <div className="flex gap-2">
        <button type="button" autoFocus className="px-2 py-1 rounded border border-[var(--color-border)] hover:bg-[var(--color-hover)]" onClick={onCancel}>
          Go back
        </button>
        <button type="button" className="px-2 py-1 rounded bg-red-600 text-white hover:bg-red-700" onClick={onConfirm}>
          Save with high-risk steps
        </button>
      </div>
    </section>
  );
}
