import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

/**
 * Opens a native OS folder picker dialog via a dedicated Rust IPC command.
 * This avoids z-order and focus issues that occur when the JS dialog plugin
 * tries to open a native sheet behind a CSS modal overlay.
 */
export async function pickFolder(title?: string): Promise<string | null> {
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
