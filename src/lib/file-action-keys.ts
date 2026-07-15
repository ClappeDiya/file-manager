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
