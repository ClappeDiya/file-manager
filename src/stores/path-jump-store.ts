/**
 * Path Jump Store — state for the Instant Jump picker (Cmd+Shift+O).
 *
 * Session-only: the picker is a transient modal, not a layout preference.
 * Kept minimal — the actual result list is stored in component state
 * since it's query-dependent and there's no need to share it between
 * components.
 */
import { create } from "zustand";

interface PathJumpState {
  isOpen: boolean;
  open: () => void;
  close: () => void;
  toggle: () => void;
}

export const usePathJumpStore = create<PathJumpState>((set) => ({
  isOpen: false,
  open: () => set({ isOpen: true }),
  close: () => set({ isOpen: false }),
  toggle: () => set((s) => ({ isOpen: !s.isOpen })),
}));
