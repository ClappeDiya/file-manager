/**
 * Pure resolver for the file-action keyboard shortcuts.
 *
 * The command palette (`command-palette.tsx`) and the auto-generated
 * keyboard cheat sheet have always advertised two shortcuts on the
 * Rename and "Move to Trash" entries:
 *
 *   - `F2`               → Rename
 *   - `⌘/Ctrl+Backspace` → Move to Trash
 *
 * …but the global keydown handler in `file-manager.tsx` never bound
 * those keys to anything, so pressing them did nothing. The hints
 * were a promise the app broke. This module is the missing piece: it
 * maps a raw keystroke to the file action it should trigger.
 *
 * It is deliberately a *pure* predicate (no DOM, no store, no IPC) so
 * the safety-critical branches can be exhaustively unit-tested. The
 * handler stays the single source of truth for *what* each action
 * does (it already owns `handleRename` / `handleDelete`); this helper
 * only decides *which* gesture, if any, a keystroke represents.
 *
 * Safety rules — a destructive shortcut earns its caution:
 *   - Never fires while a text field is focused, so the keys keep
 *     their native meaning while the user types in search / filter /
 *     rename / AI prompts (on macOS, ⌘+Backspace = delete-to-line-
 *     start, which must be preserved).
 *   - Bare Backspace — the most-pressed key in any text field — never
 *     trashes files; only the explicit ⌘/Ctrl+Backspace or the
 *     dedicated Delete key do.
 *   - Shift / Alt variants are no-ops. In particular Shift+Delete
 *     (the Windows "permanent delete" gesture) is intentionally left
 *     unbound — permanent deletion stays behind the context menu's
 *     guarded, double-confirmed path, never a single keystroke.
 */

/** The file actions a keystroke can resolve to. */
export type FileActionKey = "rename" | "trash";

/**
 * Structural subset of `KeyboardEvent` the resolver needs. The live
 * handler passes the real event; tests pass plain objects.
 */
export interface KeyGesture {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/**
 * Resolve a keystroke to a file action, or `null` if the keystroke is
 * not a (safe) file-action shortcut in the current context.
 *
 * @param e                A keystroke (subset of `KeyboardEvent`).
 * @param textEntryFocused Whether an input / textarea / contenteditable
 *                         currently has focus. When `true`, the resolver
 *                         always returns `null` so typing is never hijacked.
 */
export function resolveFileActionKey(
  e: KeyGesture,
  textEntryFocused: boolean,
): FileActionKey | null {
  // Typing always wins: a focused text field keeps every key.
  if (textEntryFocused) return null;

  // Rename — unmodified F2 (universal across Explorer / Finder / Nautilus).
  if (e.key === "F2" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
    return "rename";
  }

  // Move to Trash — the dedicated Delete key (Windows / Linux), with no
  // modifier. Shift+Delete (permanent) and Alt+Delete are deliberately
  // excluded; they fall through to `null`.
  if (e.key === "Delete" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
    return "trash";
  }

  // Move to Trash — ⌘+Backspace (macOS) / Ctrl+Backspace (Windows/Linux),
  // exactly as advertised in the palette. Bare Backspace is excluded so
  // everyday text-editing never deletes files; Shift/Alt variants too.
  if (e.key === "Backspace" && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
    return "trash";
  }

  return null;
}

export type FileTransferKey = "select_all" | "copy" | "cut" | "paste" | "copy_to_pane" | "move_to_pane";

/**
 * ⌘/Ctrl+Shift+N → New Folder (Finder convention). Advertised on the
 * context-menu entry, palette command and cheat sheet for ages, but no
 * keydown handler ever implemented it — pressing it did nothing (the
 * same keystroke was hidden-bound to the Connections panel, a genuine
 * double-binding bug: one gesture, two advertised owners, one fired).
 * Bound in the active pane's keydown handler via this predicate.
 *
 * Case-insensitive on purpose: on macOS, AppKit reports the *unshifted*
 * character for Cmd-modified key events, so ⌘⇧N arrives as `e.key ===
 * "n"`, not `"N"`. Every shift-letter shortcut in the app was dead in
 * the installed build until this normalization was applied.
 */
export function resolveNewFolderKey(
  e: KeyGesture,
  textEntryFocused: boolean,
): boolean {
  if (textEntryFocused) return false;
  return (
    (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === "n"
  );
}

/**
 * Resolve selection / transfer shortcuts: ⌘/Ctrl+A, ⌘/Ctrl+C (copy),
 * ⌘/Ctrl+X (cut), ⌘/Ctrl+V (paste), F5 (copy to other pane) and F6 (move
 * to other pane).
 */
export function resolveFileTransferKey(
  e: KeyGesture,
  textEntryFocused: boolean,
): FileTransferKey | null {
  if (textEntryFocused) return null;
  const mod = (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey;
  const key = e.key.toLowerCase();
  if (mod && key === "a") return "select_all";
  if (mod && key === "c") return "copy";
  if (mod && key === "x") return "cut";
  if (mod && key === "v") return "paste";
  const bare = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
  if (bare && e.key === "F5") return "copy_to_pane";
  if (bare && e.key === "F6") return "move_to_pane";
  return null;
}

/**
 * Go to the parent folder: Alt+↑ (Windows Explorer, Linux file managers),
 * ⌘+↑ (Finder) and bare Backspace (Explorer, RcloneView). Never while a
 * text field has focus, so Backspace keeps deleting characters. Going up
 * changes nothing on disk, so the bare key is safe here (unlike trashing).
 */
export function resolveParentFolderKey(e: KeyGesture, textEntryFocused: boolean): boolean {
  if (textEntryFocused) return false;
  if (e.key === "ArrowUp" && !e.shiftKey && !e.ctrlKey && (e.altKey !== e.metaKey)) return true;
  return e.key === "Backspace" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
}

/** ⌘/Ctrl+N (no Shift: ⌘⇧N is New Folder) → New Window. */
export function resolveNewWindowKey(e: KeyGesture, textEntryFocused: boolean): boolean {
  if (textEntryFocused) return false;
  return (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "n";
}

/** Whether `el` is a text field (input, textarea, contenteditable). */
export function isTextEntry(el: Element | null): boolean {
  const h = el as HTMLElement | null;
  return h?.tagName === "INPUT" || h?.tagName === "TEXTAREA" || h?.isContentEditable === true;
}
