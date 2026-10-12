import { create } from "zustand";

export type FileClipboardMode = "copy" | "cut";

interface FileClipboardState {
  paths: string[];
  mode: FileClipboardMode;
  set: (paths: string[], mode: FileClipboardMode) => void;
  clear: () => void;
}

/** In-app file clipboard backing Copy / Cut / Paste (not persisted). */
export const useFileClipboardStore = create<FileClipboardState>((set) => ({
  paths: [],
  mode: "copy",
  set: (paths, mode) => set({ paths: [...paths], mode }),
  clear: () => set({ paths: [], mode: "copy" }),
}));
