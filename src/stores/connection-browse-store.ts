/**
 * "Open in the connection browser": history rows (sync and backup runs) that
 * end on a saved connection ask the Connection Manager to connect and show
 * a folder. The file manager opens the manager when a request is pending and
 * the connection panel consumes it. Not persisted.
 */
import { create } from "zustand";

export interface BrowseRequest {
  connectionId: string;
  path: string;
}

interface ConnectionBrowseState {
  request: BrowseRequest | null;
  open: (connectionId: string, path: string) => void;
  /** Take the pending request (the panel calls this once it acts on it). */
  consume: () => BrowseRequest | null;
}

export const useConnectionBrowse = create<ConnectionBrowseState>((set, get) => ({
  request: null,
  open: (connectionId, path) => set({ request: { connectionId, path } }),
  consume: () => {
    const r = get().request;
    if (r) set({ request: null });
    return r;
  },
}));
