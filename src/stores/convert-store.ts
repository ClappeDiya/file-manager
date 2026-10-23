/**
 * File conversion (`convert_commands.rs`): images (resize / change format)
 * and office documents (LibreOffice). The dialog's open state lives here.
 */
import { create } from "zustand";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import type { DocumentFormat, ImageFormat } from "@/stores/automation-store";

/** `convert_engine::ImageSpec` */
export interface ImageSpec {
  format: ImageFormat | null;
  max_width: number | null;
  max_height: number | null;
  quality: number | null;
}

export interface ConvertOutcome {
  input: string;
  output: string | null;
  error: string | null;
}

export interface ConvertCapabilities {
  image_formats: ImageFormat[];
  document_formats: DocumentFormat[];
  office: boolean;
}

export const convert = {
  capabilities: () => tauriInvokeSafe<ConvertCapabilities>("convert_capabilities", undefined, { image_formats: [], document_formats: [], office: false }),
  images: (paths: string[], spec: ImageSpec, destDir: string | null) => tauriInvoke<ConvertOutcome[]>("convert_images", { paths, spec, destDir }),
  documents: (paths: string[], to: DocumentFormat, destDir: string | null) => tauriInvoke<ConvertOutcome[]>("convert_documents", { paths, to, destDir }),
};

const IMAGE = /\.(png|jpe?g|webp|gif|bmp|tiff?)$/i;
const DOCUMENT = /\.(docx?|odt|rtf|txt|html?|xlsx?|ods|csv|pptx?|odp)$/i;
export const isConvertibleImage = (name: string) => IMAGE.test(name);
export const isConvertibleDocument = (name: string) => DOCUMENT.test(name);

/** What the selection can be converted as: all images, all documents, or neither. */
export function convertKind(paths: string[]): "image" | "document" | null {
  if (paths.length === 0) return null;
  if (paths.every(isConvertibleImage)) return "image";
  if (paths.every(isConvertibleDocument)) return "document";
  return null;
}

interface ConvertDialogState {
  paths: string[];
  open: (paths: string[]) => void;
  close: () => void;
}

export const useConvertDialog = create<ConvertDialogState>((set) => ({
  paths: [],
  open: (paths) => set({ paths }),
  close: () => set({ paths: [] }),
}));
