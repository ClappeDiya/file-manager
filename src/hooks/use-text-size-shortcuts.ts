import { useEffect } from "react";
import { useUIStore } from "@/stores/ui-store";

type KeyLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey">;

/**
 * Text size step for a key press: Ctrl/Cmd + "+" larger, Ctrl/Cmd + "−"
 * smaller, Ctrl/Cmd + 0 default; `null` for any other key.
 *
 * Ctrl/Cmd + "=" (unshifted) stays bound to "Compare files", so only the
 * shifted "+" (and the keypad +) enlarge text. Alt combinations are left
 * alone (they type characters on some layouts).
 */
export function textSizeStepForKey(e: KeyLike): -1 | 0 | 1 | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  switch (e.key) {
    case "+":
      return 1;
    case "-":
    case "_":
      return -1;
    case "0":
      return 0;
    default:
      return null;
  }
}

/** Binds the text size shortcuts for the whole window (mounted once). The
 *  webview's own zoom keys are off in Tauri, so these replace them. */
export function useTextSizeShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const step = textSizeStepForKey(e);
      if (step === null) return;
      e.preventDefault();
      useUIStore.getState().stepFontScale(step);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
