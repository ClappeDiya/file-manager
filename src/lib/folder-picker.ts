import { isTauriAvailable } from "@/hooks/use-tauri";

/**
 * Opens a native OS folder picker dialog (Tauri) or falls back to
 * a browser prompt in dev mode.
 */
export async function pickFolder(title?: string): Promise<string | null> {
  if (isTauriAvailable()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      directory: true,
      title: title ?? "Select Folder",
    });
    return typeof selected === "string" ? selected : null;
  }

  // Browser dev-mode fallback: prompt user to type a path
  const path = window.prompt(title ?? "Enter folder path:");
  return path && path.trim() ? path.trim() : null;
}
