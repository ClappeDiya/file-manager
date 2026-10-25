/** Whether the Send feedback dialog (Help > Send feedback) is showing. Shared by the command palette and Settings. Not persisted. */
import { create } from "zustand";

interface FeedbackState {
  open: boolean;
  setOpen: (open: boolean) => void;
}

export const useFeedbackDialog = create<FeedbackState>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));
