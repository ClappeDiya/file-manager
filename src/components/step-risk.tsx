/**
 * Per-step risk in the rule editor and workflow builder (Advanced). The
 * classification itself lives in Rust (`automation_engine::risk`, one place
 * for the badge, the editor's confirmation and the tests); this file only
 * asks for it and draws it. Saving a rule that contains a High step needs an
 * explicit answer in an inline dialog.
 */
import { useEffect, useMemo, useState } from "react";
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import type { RiskLevel, RuleAction, StepRisk } from "@/stores/automation-store";

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

/** Ask Rust how risky each of these actions is (`automation_action_risks`); one answer per action, in order. */
export async function fetchRisks(actions: RuleAction[]): Promise<StepRisk[]> {
  if (actions.length === 0) return [];
  const risks = await tauriInvokeSafe<StepRisk[]>("automation_action_risks", { actions }, []);
  return Array.isArray(risks) && risks.length === actions.length ? risks : [];
}

/**
 * A lookup from an action to its risk, refreshed when the actions change.
 * Unknown (outside the app, or not yet answered) = no badge.
 */
export function useRiskLookup(actions: RuleAction[]): (a: RuleAction) => StepRisk | undefined {
  const key = JSON.stringify(actions);
  const [table, setTable] = useState<Map<string, StepRisk>>(new Map());
  useEffect(() => {
    let live = true;
    const list = JSON.parse(key) as RuleAction[];
    void fetchRisks(list).then((risks) => live && setTable(new Map(list.map((a, i) => [JSON.stringify(a), risks[i]] as const).filter(([, r]) => r))));
    return () => {
      live = false;
    };
  }, [key]);
  return useMemo(() => (a: RuleAction) => table.get(JSON.stringify(a)), [table]);
}

/** The High steps among `actions`, numbered from 1 as the editor shows them. */
export function highRiskSteps(actions: RuleAction[], risks: StepRisk[]): { step: number; risk: StepRisk }[] {
  return risks.flatMap((risk, i) => (risk.level === "high" && actions[i] ? [{ step: i + 1, risk }] : []));
}

/** Inline confirmation shown before saving a rule with High-risk steps (not `window.confirm`). */
export function RiskConfirm({ steps, onConfirm, onCancel }: { steps: { step: number; risk: StepRisk }[]; onConfirm: () => void; onCancel: () => void }) {
  return (
    <section role="alertdialog" aria-labelledby="risk-confirm-title" className="rounded border border-red-500/60 bg-red-500/5 p-3 text-xs space-y-2" data-testid="risk-confirm">
      <p id="risk-confirm-title" className="font-semibold text-red-700 dark:text-red-300">
        This rule has {steps.length === 1 ? "a high-risk step" : `${steps.length} high-risk steps`}. Save it anyway?
      </p>
      <ul className="list-disc pl-5">
        {steps.map((s) => (
          <li key={s.step}>
            Action {s.step}: {s.risk.reason}
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
