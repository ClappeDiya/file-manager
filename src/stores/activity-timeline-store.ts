/**
 * Activity Timeline Store
 *
 * Session-only Zustand store controlling the visibility of the Activity
 * Timeline panel. Intentionally not persisted: the panel is an on-demand
 * pull surface (like the command palette), not a layout preference the
 * user would want restored on next launch.
 *
 * The panel itself fetches data directly from the unified OperationLedger
 * via `ledger_recent` when it mounts, so this store only needs to track
 * open/closed state. Keeping it minimal avoids duplicating ledger data in
 * the frontend and keeps the surface area small.
 */
import { create } from "zustand";

interface ActivityTimelineState {
  panelOpen: boolean;
  openPanel: () => void;
  closePanel: () => void;
  togglePanel: () => void;
}

export const useActivityTimelineStore = create<ActivityTimelineState>((set) => ({
  panelOpen: false,
  openPanel: () => set({ panelOpen: true }),
  closePanel: () => set({ panelOpen: false }),
  togglePanel: () => set((state) => ({ panelOpen: !state.panelOpen })),
}));
