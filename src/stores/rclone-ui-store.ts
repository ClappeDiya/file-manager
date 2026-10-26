/**
 * Whether the rclone remote manager is showing inside the Connection
 * Manager, and in which mode. Shared by the connection panel, its forms and
 * the command palette. Not persisted.
 */
import { create } from "zustand";

interface RcloneUiState {
  managerOpen: boolean;
  /** Open straight into "new remote". */
  startInCreate: boolean;
  /** Backend to preselect when creating. */
  createType: string | null;
  openManager: (opts?: { create?: boolean; type?: string | null }) => void;
  closeManager: () => void;
}

export const useRcloneUi = create<RcloneUiState>((set) => ({
  managerOpen: false,
  startInCreate: false,
  createType: null,
  openManager: (opts) => set({ managerOpen: true, startInCreate: !!opts?.create || !!opts?.type, createType: opts?.type ?? null }),
  closeManager: () => set({ managerOpen: false, startInCreate: false, createType: null }),
}));
