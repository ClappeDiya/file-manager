import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

type FolderPickerImpl = (title: string) => Promise<string | null>;
let customPicker: FolderPickerImpl | null = null;

/** Let a host (the ufop-server web UI) supply its own folder browser. */
export function setFolderPickerImpl(fn: FolderPickerImpl | null) {
  customPicker = fn;
}

/**
 * Opens a native OS folder picker dialog via a dedicated Rust IPC command.
 * This avoids z-order and focus issues that occur when the JS dialog plugin
 * tries to open a native sheet behind a CSS modal overlay.
 */
export async function pickFolder(title?: string): Promise<string | null> {
  if (customPicker) return customPicker(title ?? "Select Folder");
  if (isTauriAvailable()) {
    try {
      const result = await tauriInvoke<string | null>("pick_folder", {
        title: title ?? "Select Folder",
      });
      return result ?? null;
    } catch (err) {
      console.error("Folder picker IPC failed:", err);
      // Don't fall through to prompt — show the error clearly
      return null;
    }
  }

  // Browser dev-mode fallback: prompt user to type a path
  const path = window.prompt(title ?? "Enter folder path:");
  return path && path.trim() ? path.trim() : null;
}

/** Native multi-file picker; returns [] when cancelled or outside Tauri. */
export async function pickFiles(title?: string): Promise<string[]> {
  if (!isTauriAvailable()) return [];
  try {
    return (await tauriInvoke<string[]>("pick_files", { title: title ?? "Select Files" })) ?? [];
  } catch (err) {
    console.error("File picker IPC failed:", err);
    return [];
  }
}

/**
 * Native "save as" dialog; null when cancelled. Outside Tauri the browser
 * prompts for a path (dev mode) unless a host supplied a folder picker, in
 * which case the folder is joined with `defaultName`.
 */
export async function pickSaveFile(title: string, defaultName: string, extensions: string[] = []): Promise<string | null> {
  if (isTauriAvailable()) {
    try {
      return (await tauriInvoke<string | null>("pick_save_file", { title, defaultName, extensions })) ?? null;
    } catch (err) {
      console.error("Save dialog IPC failed:", err);
      return null;
    }
  }
  if (customPicker) {
    const dir = await customPicker(title);
    return dir ? `${dir.replace(/[\\/]+$/, "")}/${defaultName}` : null;
  }
  const path = window.prompt(title, defaultName);
  return path && path.trim() ? path.trim() : null;
}
