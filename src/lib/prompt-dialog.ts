/**
 * Promise-based text prompt. `window.prompt` is unsupported in the Tauri
 * macOS webview (returns null without showing UI), so callers use this and
 * `<PromptDialogHost />` renders the modal.
 */
export interface PromptRequest {
  message: string;
  defaultValue: string;
  resolve: (value: string | null) => void;
}

let listener: ((req: PromptRequest | null) => void) | null = null;

export function setPromptListener(fn: ((req: PromptRequest | null) => void) | null) {
  listener = fn;
}

export function promptText(message: string, defaultValue = ""): Promise<string | null> {
  return new Promise((resolve) => {
    if (!listener) {
      resolve(typeof window !== "undefined" ? window.prompt(message, defaultValue) : null);
      return;
    }
    listener({ message, defaultValue, resolve });
  });
}

/**
 * Promise-based yes/no confirm. `window.confirm` is also unsupported in the
 * Tauri macOS webview (returns false immediately), so destructive actions
 * must go through this — `<PromptDialogHost />` renders the modal.
 */
export interface ConfirmRequest {
  message: string;
  confirmLabel: string;
  danger: boolean;
  resolve: (ok: boolean) => void;
}

let confirmListener: ((req: ConfirmRequest | null) => void) | null = null;

export function setConfirmListener(fn: ((req: ConfirmRequest | null) => void) | null) {
  confirmListener = fn;
}

export function confirmText(
  message: string,
  confirmLabel = "OK",
  danger = false,
): Promise<boolean> {
  return new Promise((resolve) => {
    if (!confirmListener) {
      resolve(typeof window !== "undefined" ? window.confirm(message) : false);
      return;
    }
    confirmListener({ message, confirmLabel, danger, resolve });
  });
}
