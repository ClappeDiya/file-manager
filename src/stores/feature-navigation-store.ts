import { create } from "zustand";
import type { FeatureTarget } from "@/components/simple-mode/feature-catalog";

type ToolTarget = Exclude<FeatureTarget, { section: string }>;
interface FeatureNavigationState {
  pending: { target: ToolTarget; token: number } | null;
  request: (target: ToolTarget) => void;
  consume: (token: number) => boolean;
}
let nextToken = 0;
/** A one-shot request survives switching back to the Files orchestrator. Never persisted. */
export const useFeatureNavigation = create<FeatureNavigationState>(
  (set, get) => ({
    pending: null,
    request: (target) => set({ pending: { target, token: ++nextToken } }),
    consume: (token) => {
      if (get().pending?.token !== token) return false;
      set({ pending: null });
      return true;
    },
  }),
);
