/**
 * Safety Interlock — frontend wrapper around the Rust backend's
 * context-aware anomaly detection.
 *
 * The single public entry point is {@link assessBeforeExecute}: given an
 * operation intent and an async executor, it assesses the intent, blocks
 * on a user confirmation dialog when the assessment is medium/high risk,
 * and then either invokes the executor or returns `null` (user
 * cancelled). This is the ONE place in the frontend that any risky
 * operation should be wrapped in — DRY by design.
 *
 * Key properties:
 *
 * - **Fail-open.** If the backend errors or we're outside Tauri (browser
 *   preview), the assessment defaults to Low risk and the executor runs
 *   immediately. A broken safety layer never blocks legitimate work.
 * - **Zero cognitive load.** Low-risk operations pass straight through —
 *   the user never sees a dialog for anything within their normal
 *   pattern.
 * - **Single global dialog.** The store + global dialog pattern means
 *   callers don't import any UI — they just call this helper and `await`
 *   the result.
 */
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import { useSafetyStore } from "@/stores/safety-store";

/**
 * The `kind` values the Rust FS engine writes to the operation ledger, as
 * emitted by `src-tauri/src/commands/file_ops_commands.rs`.
 *
 * `OperationIntent.kind` MUST be one of these for any fs intent. The interlock
 * uses the kind verbatim as its baseline lookup key (`WHERE kind = ?`), so a
 * kind the engine never records matches zero rows and yields an empty baseline.
 *
 * These two vocabularies drifted once already: the UI sent `"delete"` and
 * `"purge"` while the engine recorded `"delete_trash"` and `"delete_permanent"`.
 * Every delete assessment therefore ran against a permanently empty baseline —
 * so the interlock told the user "first-ever delete of this kind on this device"
 * on every single delete of 25+ files, and its adaptive ratio logic never once
 * ran for a delete. Naming the kinds here keeps the intent side single-sourced;
 * `fs-intent-kinds.test.ts` pins the exact strings against the Rust writer.
 */
export const FS_INTENT_KIND = {
  move: "move",
  deleteTrash: "delete_trash",
  deletePermanent: "delete_permanent",
} as const;

/** Wire-format mirror of `safety::OperationIntent` in Rust. */
export interface OperationIntent {
  engine: string;
  kind: string;
  affected_files: number;
  total_bytes: number;
  subject_path: string | null;
  summary: string | null;
}

export type RiskLevel = "low" | "medium" | "high";

/** Wire-format mirror of `safety::RiskAssessment` in Rust. */
export interface RiskAssessment {
  level: RiskLevel;
  reasons: string[];
  baseline_sample_size: number;
  baseline_median_files: number;
  baseline_median_bytes: number;
  intent_hash: string;
  requires_confirmation: boolean;
}

/**
 * Ask the backend to assess an operation. Always resolves — errors are
 * converted to a Low assessment so a broken backend cannot block work.
 */
export async function assessIntent(
  intent: OperationIntent,
): Promise<RiskAssessment> {
  const fallback: RiskAssessment = {
    level: "low",
    reasons: ["safety: interlock unavailable, defaulting to low risk"],
    baseline_sample_size: 0,
    baseline_median_files: 0,
    baseline_median_bytes: 0,
    intent_hash: "fallback",
    requires_confirmation: false,
  };
  return await tauriInvokeSafe<RiskAssessment>(
    "safety_assess_intent",
    { intent },
    fallback,
  );
}

/**
 * Record the user's explicit approval so the confirmation lands in the
 * operation ledger. Best-effort — failures are swallowed because the
 * core operation has already been approved in the UI.
 */
export async function confirmIntent(
  intent: OperationIntent,
  assessment: RiskAssessment,
): Promise<void> {
  await tauriInvokeSafe<boolean>(
    "safety_confirm_intent",
    {
      intent,
      intentHash: assessment.intent_hash,
      level: assessment.level,
    },
    false,
  );
}

/**
 * Wrap any risky async operation in a Safety Interlock check. Low-risk
 * operations pass through immediately; medium/high-risk operations pause
 * on a confirmation dialog until the user answers.
 *
 * Returns the executor's result on success, or `null` if the user
 * rejected the confirmation. The executor is never invoked on rejection.
 *
 * @example
 * ```ts
 * const result = await assessBeforeExecute(
 *   {
 *     engine: "sync",
 *     kind: "mirror",
 *     affected_files: plan.total_files,
 *     total_bytes: plan.total_bytes,
 *     subject_path: plan.source,
 *     summary: `Mirror ${plan.source} → ${plan.destination}`,
 *   },
 *   () => tauriInvoke("sync_run_mirror", { planId }),
 * );
 * if (result === null) return;  // user aborted
 * ```
 */
export async function assessBeforeExecute<T>(
  intent: OperationIntent,
  execute: () => Promise<T>,
): Promise<T | null> {
  const assessment = await assessIntent(intent);

  if (!assessment.requires_confirmation) {
    // Low-risk: execute immediately, no UI disruption.
    return await execute();
  }

  // Medium/high-risk: block on the global dialog until the user answers.
  const approved = await new Promise<boolean>((resolve) => {
    useSafetyStore.getState().present({ intent, assessment, resolve });
  });

  if (!approved) return null;

  // Fire-and-forget audit log entry; don't hold up the user.
  void confirmIntent(intent, assessment);

  return await execute();
}
