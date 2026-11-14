/**
 * Which mount screens are open: the Mounts list and the mount dialog (new, for
 * a given connection, or editing a saved mount). Shared by the toolbar, the
 * command palette, the connection manager and the tray's refresh events.
 * Not persisted.
 */
import { create } from "zustand";
import type { MountInfo } from "@/lib/mount-options";

interface MountsUiState {
  panelOpen: boolean;
  dialogOpen: boolean;
  /** Connection to preselect in the dialog. */
  dialogConnectionId: string | null;
  /** Saved mount being edited (null = a new mount). */
  editing: MountInfo | null;
  /** Bumped when mounts change so open lists reload. */
  refreshTick: number;
  openPanel: () => void;
  closePanel: () => void;
  togglePanel: () => void;
  openDialog: (opts?: {
    connectionId?: string | null;
    editing?: MountInfo | null;
  }) => void;
  closeDialog: () => void;
  bump: () => void;
}

export const useMountsUi = create<MountsUiState>((set) => ({
  panelOpen: false,
  dialogOpen: false,
  dialogConnectionId: null,
  editing: null,
  refreshTick: 0,
  openPanel: () => set({ panelOpen: true }),
  closePanel: () => set({ panelOpen: false }),
  togglePanel: () => set((s) => ({ panelOpen: !s.panelOpen })),
  openDialog: (opts) =>
    set({
      dialogOpen: true,
      dialogConnectionId: opts?.connectionId ?? null,
      editing: opts?.editing ?? null,
    }),
  closeDialog: () =>
    set({ dialogOpen: false, dialogConnectionId: null, editing: null }),
  bump: () => set((s) => ({ refreshTick: s.refreshTick + 1 })),
}));
