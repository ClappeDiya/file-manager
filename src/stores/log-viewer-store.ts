/** Whether the in-app log viewer (Advanced) is showing. Shared by the toolbar, the command palette and Settings → Logging. Not persisted. */
import { create } from "zustand";

interface LogViewerState {
  open: boolean;
  setOpen: (open: boolean) => void;
  toggle: () => void;
}

export const useLogViewer = create<LogViewerState>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
  toggle: () => set((s) => ({ open: !s.open })),
}));
