import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";

interface FeaturePeekState {
  /** Set of feature ids the user has dismissed (or auto-dismissed by viewing). */
  dismissed: string[];
  /** True if the user has dismissed this feature peek (or never opted-in). */
  isDismissed: (id: string) => boolean;
  /** Mark a feature peek as dismissed so it never appears again. */
  dismiss: (id: string) => void;
  /** Reset all dismissals — useful from Settings if a user wants the tour back. */
  reset: () => void;
}

export const useFeaturePeekStore = create<FeaturePeekState>()(
  persist(
    (set, get) => ({
      dismissed: [],
      isDismissed: (id) => get().dismissed.includes(id),
      dismiss: (id) =>
        set((state) =>
          state.dismissed.includes(id) ? state : { dismissed: [...state.dismissed, id] },
        ),
      reset: () => set({ dismissed: [] }),
    }),
    {
      name: "ufop-feature-peeks",
      storage: createJSONStorage(() => localStorage),
    },
  ),
);
