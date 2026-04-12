/**
 * KeyboardCheatSheet
 *
 * Pure derived overlay that lists every command-palette entry with a
 * bound keyboard shortcut, grouped by category. Triggered by pressing
 * `?` (Shift+/) when no text input is focused — the universal
 * discoverability convention used by GitHub, Gmail, Slack, Linear,
 * and Notion.
 *
 * Design principles applied:
 *
 * - **Pure derived view**: reads the same `CommandItem[]` array that
 *   already drives the command palette. No parallel list, no manual
 *   maintenance — every future keyboard shortcut shows up
 *   automatically the moment its palette entry is added. This is the
 *   DRY property that makes the cheat sheet a net win in
 *   maintenance cost instead of a burden.
 *
 * - **Self-updating**: iter 10's `Cmd+1..9`, iter 11's
 *   `Cmd+Shift+E`, iter 12's `Cmd+Shift+X`, iter 13's
 *   `Cmd+Shift+F`, iter 14's `Cmd+Shift+V`, iter 15's `Cmd+Shift+K`
 *   and iter 16's `Cmd+Shift+C` all appear here without any
 *   per-shortcut wiring on this side, because each one already
 *   registered its palette entry during its own iteration.
 *
 * - **Zero new state**: the only state is a local `open` flag owned
 *   by the parent. No Zustand entry, no persistence, no IPC.
 *
 * - **Defensive input-focus guard**: the `?` trigger ignores the
 *   keystroke when the user is typing into an input/textarea/
 *   contenteditable, otherwise every question mark typed anywhere
 *   in the app would pop the overlay.
 *
 * - **Escape to close**: matches the command palette's own modal
 *   dismissal pattern so users don't have to learn two different
 *   "get out of this modal" gestures.
 */
import { useEffect, useMemo } from "react";
import { cn } from "@ufop/ui-components";
import { X } from "lucide-react";
import type { CommandItem } from "./command-palette";
import { groupShortcutCommandsByCategory } from "@/lib/keyboard-shortcut-groups";

interface KeyboardCheatSheetProps {
  commands: CommandItem[];
  open: boolean;
  onClose: () => void;
}

export function KeyboardCheatSheet({
  commands,
  open,
  onClose,
}: KeyboardCheatSheetProps) {
  const groups = useMemo(
    () => groupShortcutCommandsByCategory(commands),
    [commands],
  );

  // Escape closes the overlay. We deliberately attach the listener
  // only while `open` is true so the cheat sheet has zero runtime
  // cost when closed.
  useEffect(() => {
    if (!open) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className={cn(
        "fixed inset-0 z-50 flex items-start justify-center pt-20",
        "bg-black/50 backdrop-blur-sm",
      )}
      onClick={onClose}
      data-testid="keyboard-cheat-sheet-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="Keyboard shortcuts"
    >
      <div
        className={cn(
          "max-h-[80vh] w-[min(720px,90vw)] overflow-y-auto rounded-lg shadow-2xl",
          "bg-[var(--color-surface)] border border-[var(--color-border)]",
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className={cn(
            "sticky top-0 flex items-center justify-between px-5 py-3",
            "bg-[var(--color-surface)] border-b border-[var(--color-border)]",
          )}
        >
          <h2 className="text-base font-semibold text-[color:var(--color-text)]">
            Keyboard Shortcuts
          </h2>
          <button
            className={cn(
              "h-7 w-7 flex items-center justify-center rounded",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "hover:bg-[var(--color-hover)]",
            )}
            onClick={onClose}
            aria-label="Close keyboard shortcuts"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="px-5 py-4 space-y-6">
          {groups.length === 0 ? (
            <p className="text-[length:var(--font-size-sm)] text-[color:var(--color-text-tertiary)]">
              No keyboard shortcuts are registered.
            </p>
          ) : (
            groups.map(({ category, items }) => (
              <section key={category}>
                <h3 className="mb-2 text-[length:var(--font-size-xs)] font-semibold uppercase tracking-wide text-[color:var(--color-text-tertiary)]">
                  {category}
                </h3>
                <ul className="divide-y divide-[var(--color-border)]">
                  {items.map((cmd) => (
                    <li
                      key={cmd.id}
                      className="flex items-start justify-between gap-4 py-2"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[length:var(--font-size-sm)] text-[color:var(--color-text)]">
                          {cmd.label}
                        </div>
                        {cmd.description && (
                          <div className="truncate text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
                            {cmd.description}
                          </div>
                        )}
                        {/* Iter 48: secondary shortcut hints render
                            below the description so the keyboard
                            variant is discoverable via `?` even
                            though it's hidden from the ⌘K palette. */}
                        {cmd.additionalShortcuts?.map((extra) => (
                          <div
                            key={extra.keys}
                            className="mt-0.5 flex items-center gap-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]"
                          >
                            <kbd
                              className={cn(
                                "rounded border px-1.5 py-px",
                                "font-mono",
                                "bg-[var(--color-surface-elevated)] border-[var(--color-border)]",
                                "text-[color:var(--color-text-tertiary)]",
                              )}
                            >
                              {extra.keys}
                            </kbd>
                            <span className="truncate">{extra.hint}</span>
                          </div>
                        ))}
                      </div>
                      <kbd
                        className={cn(
                          "shrink-0 rounded border px-2 py-0.5",
                          "font-mono text-[length:var(--font-size-xs)]",
                          "bg-[var(--color-surface-elevated)] border-[var(--color-border)]",
                          "text-[color:var(--color-text)]",
                        )}
                      >
                        {cmd.shortcut}
                      </kbd>
                    </li>
                  ))}
                </ul>
              </section>
            ))
          )}
        </div>
        <div
          className={cn(
            "sticky bottom-0 px-5 py-2 text-[length:var(--font-size-xs)]",
            "text-[color:var(--color-text-tertiary)]",
            "bg-[var(--color-surface)] border-t border-[var(--color-border)]",
          )}
        >
          Press <kbd className="font-mono">?</kbd> to toggle {"\u2022"}{" "}
          <kbd className="font-mono">Esc</kbd> to close
        </div>
      </div>
    </div>
  );
}
