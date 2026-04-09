/**
 * Safety Interlock Store — pending risk assessments for the confirmation
 * dialog.
 *
 * The Safety Interlock is invoked through {@link assessBeforeExecute} in
 * `src/lib/safety.ts`. When that helper receives a medium/high risk
 * assessment, it pushes a `PendingAssessment` into this store along with
 * a resolver callback, causing the global `SafetyInterlockDialog` to
 * open. The dialog calls `approve()` or `reject()` to resume the
 * suspended caller.
 *
 * Session-only: pending confirmations are transient by design — they
 * exist for the duration of a single blocked operation and vanish when
 * the user answers the prompt.
 */
import { create } from "zustand";
import type { OperationIntent, RiskAssessment } from "@/lib/safety";

export interface PendingAssessment {
  intent: OperationIntent;
  assessment: RiskAssessment;
  /** Resolves the suspended caller — `true` to proceed, `false` to abort. */
  resolve: (approved: boolean) => void;
}

interface SafetyState {
  pending: PendingAssessment | null;
  present: (p: PendingAssessment) => void;
  approve: () => void;
  reject: () => void;
}

export const useSafetyStore = create<SafetyState>((set, get) => ({
  pending: null,
  present: (p) => set({ pending: p }),
  approve: () => {
    const p = get().pending;
    if (p) {
      p.resolve(true);
      set({ pending: null });
    }
  },
  reject: () => {
    const p = get().pending;
    if (p) {
      p.resolve(false);
      set({ pending: null });
    }
  },
}));
