/**
 * clipboard
 *
 * Centralises the "write to clipboard then surface feedback" pattern
 * that was previously duplicated across five separate copy handlers
 * (path / remote-path / URL / relative-path / as-script) in
 * `file-manager.tsx`. Every one of those handlers did:
 *
 *   try { await navigator.clipboard.writeText(...); } catch {}
 *
 * — silently swallowing both the success (no confirmation toast)
 * and the failure (no error toast, no console.error, nothing). The
 * user had no way to know whether the copy worked short of pasting
 * somewhere else.
 *
 * This module funnels the five handlers through one helper that:
 *   - writes to the OS clipboard
 *   - surfaces a structured success toast with a clamped preview
 *     so the user can see at-a-glance what is now on the clipboard
 *   - surfaces a structured failure toast with the underlying error
 *     so silent denials (permission errors on web/Tauri, browser
 *     security blocks, etc.) are no longer invisible
 *
 * # Why structured toasts
 *
 * The existing UI store's `addStructuredError` is the canonical
 * notification surface (used by every backend-toast feedback path)
 * — using it here keeps every "system told me what happened" event
 * routed through one channel, so future toast-grouping or sound
 * cues apply uniformly.
 */
import { useUIStore } from "@/stores/ui-store";

/** Default preview cap. 60 chars fits comfortably in a toast body
 *  while still showing enough of typical paths to disambiguate. */
const DEFAULT_PREVIEW_MAX = 60;

/**
 * Trim a clipboard value to a length suitable for the toast body
 * and collapse internal whitespace runs so multi-line scripts read
 * cleanly as a single line. Pure: no side effects.
 *
 * - Inputs of length `≤ max` (after collapsing whitespace) are
 *   returned verbatim.
 * - Longer inputs are middle-truncated with `…` so both the start
 *   AND end remain visible (the end of a path is usually the
 *   filename, which the user often cares about most).
 * - Empty / whitespace-only inputs return an empty string so the
 *   toast caller can decide whether to suppress the preview.
 */
export function previewClipboardValue(
  value: string,
  max: number = DEFAULT_PREVIEW_MAX,
): string {
  if (max < 4) return ""; // No room for a useful preview.
  const compact = value.replace(/\s+/g, " ").trim();
  if (compact.length <= max) return compact;
  const slack = max - 1; // 1 char for the ellipsis
  const head = Math.ceil(slack / 2);
  const tail = Math.floor(slack / 2);
  return `${compact.slice(0, head)}…${compact.slice(compact.length - tail)}`;
}

/**
 * Write `value` to the OS clipboard and surface a structured toast
 * reporting success or failure. `label` is the human-readable name
 * of the thing being copied ("Copy Path", "Copy URL", etc.) — it
 * appears in the toast's `what` line so a glance at the toast tells
 * the user which copy fired.
 *
 * Resolves once the toast has been queued. Never throws — failures
 * are reported via the toast surface, not via rejection, so callers
 * can `await` it inside a fire-and-forget handler.
 */
export async function copyToClipboardWithToast(
  value: string,
  label: string,
): Promise<void> {
  const preview = previewClipboardValue(value);
  try {
    await navigator.clipboard.writeText(value);
    useUIStore.getState().addStructuredError({
      what: `${label} copied`,
      why: preview || "(empty value)",
      appDid: `Wrote ${value.length} character${value.length === 1 ? "" : "s"} to the system clipboard`,
      userAction: "Paste with Cmd/Ctrl+V where you need it",
    });
  } catch (err) {
    useUIStore.getState().addStructuredError({
      what: `${label} failed`,
      why:
        err instanceof Error && err.message
          ? err.message
          : "The clipboard API was unavailable or denied access",
      appDid: "Nothing was placed on the clipboard",
      userAction:
        "Check OS / browser clipboard permissions, then try the copy again",
    });
  }
}
