import { useEffect, useMemo, useState } from "react";
import { isTauriAvailable, tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import type { AutomationRule, RiskLevel, RuleAction, StepRisk } from "@/stores/automation-store";


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


/** A High-risk step of a rule, as `automation_rule_confirmations` names it (`Action 2`, `Step Wipe`). */
export interface HighRiskStep {
  step: string;
  level: RiskLevel;
  reason: string;
}


/**
 * The steps of `rule` that need an explicit yes before saving, from Rust
 * (the same labels the workflow graph uses). Outside the app there is no
 * backend to ask and the save fails later anyway. Inside it, a failed or
 * malformed answer is never read as "all clear": the user is asked to
 * confirm the whole rule instead.
 */
export async function fetchHighRiskSteps(rule: AutomationRule): Promise<HighRiskStep[]> {
  if (!isTauriAvailable()) return [];
  try {
    const steps = await tauriInvoke<HighRiskStep[]>("automation_rule_confirmations", { rule });
    if (Array.isArray(steps) && steps.every((x) => x && typeof x.step === "string" && typeof x.reason === "string")) return steps;
  } catch {
    // Falls through to the unchecked answer below.
  }
  return [{ step: "This rule", level: "high", reason: "Its steps could not be checked for risk, so it may remove data or run programs" }];
}
