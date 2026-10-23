/** State for the "Copy to several folders" dialog. */
import { create } from "zustand";
import { useFileManagerStore } from "@/stores/file-manager-store";

/** Mirrors `DestinationOutcome` in file_ops_commands.rs. */
export interface DestinationOutcome {
  dest_dir: string;
  result: { dest_paths: string[]; undo_id: string } | null;
  error: string | null;
}

interface CopyToManyState {
  sources: string[];
  candidates: string[];
  open: (sources: string[], candidates: string[]) => void;
  close: () => void;
}

export const useCopyToManyStore = create<CopyToManyState>((set) => ({
  sources: [],
  candidates: [],
  open: (sources, candidates) => set({ sources, candidates }),
  close: () => set({ sources: [], candidates: [] }),
}));

/** Distinct, non-empty folders other than the one the files are in. */
export function destinationCandidates(current: string, other: string, favorites: string[]): string[] {
  return [...new Set([other, ...favorites])].filter((p) => p && p !== current);
}

/** Open the dialog for a pane's selection. */
export function openCopyToMany(sources: string[], currentPath: string, otherPath: string) {
  const favorites = useFileManagerStore.getState().favorites.map((f) => f.path);
  useCopyToManyStore.getState().open(sources, destinationCandidates(currentPath, otherPath, favorites));
}
