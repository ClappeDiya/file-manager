/**
 * Which folders the compare window is comparing. Session-only: the window
 * opens from "Compare panes" (dual-pane toolbar, toolbar, command palette)
 * or "Compare with…" (Connections) and reads its two sides from here.
 */
import { create } from "zustand";
import type { CompareSide } from "@/lib/folder-compare";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";

export interface CompareRequest {
  left: CompareSide;
  right: CompareSide;
  /** Bumped on every open, so re-opening the same pair compares again. */
  nonce: number;
}

interface CompareState {
  request: CompareRequest | null;
  open: (left: CompareSide, right: CompareSide) => void;
  /** Compare the active folders of the left and right panes. */
  openPanes: () => void;
  close: () => void;
}

let nonce = 0;

/** Whether a pane path is a local folder (panes can show `scheme://` locations). */
const isLocalPath = (p: string) => !p.includes("://");

export const useCompareStore = create<CompareState>((set, get) => ({
  request: null,
  open: (left, right) => set({ request: { left, right, nonce: ++nonce } }),
  openPanes: () => {
    const fm = useFileManagerStore.getState();
    const left = fm.getActiveTab(0)?.path;
    const right = fm.getActiveTab(1)?.path;
    if (!left || !right || !isLocalPath(left) || !isLocalPath(right)) {
      useUIStore.getState().addStructuredError({
        what: "Compare panes needs two local folders",
        why: "One of the panes is not showing a folder on this computer",
        appDid: "Did not start a comparison",
        userAction: "Open a local folder in each pane, or use Compare with… in Connections for a remote folder",
      });
      return;
    }
    get().open({ type: "local", path: left }, { type: "local", path: right });
  },
  close: () => set({ request: null }),
}));
