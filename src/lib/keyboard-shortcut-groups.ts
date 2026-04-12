/**
 * Pure helpers for the keyboard cheat sheet overlay (iter 17).
 *
 * Kept in `src/lib/` (rather than alongside the component file) so
 * the React Fast Refresh rule `react-refresh/only-export-components`
 * stays satisfied — the cheat sheet component file can export only
 * its component without mixing in non-component exports.
 *
 * The grouping is a pure function over a `CommandItem[]`, so it can
 * be unit-tested without mounting React. Every future iteration
 * that adds a palette entry with a `shortcut` field automatically
 * flows through this helper — zero per-shortcut wiring on the
 * cheat sheet side.
 */
import type { CommandItem } from "@/components/command-palette";

export interface ShortcutCategoryGroup {
  category: string;
  items: CommandItem[];
}

/**
 * Filter commands to those with a visible keyboard shortcut and
 * group them by category, preserving the insertion order of
 * categories (first-seen wins) and the order of items within each
 * category. The first-seen ordering matches how the user
 * experiences the palette itself, so the cheat sheet stays
 * consistent with the rest of the app.
 */
export function groupShortcutCommandsByCategory(
  commands: CommandItem[],
): ShortcutCategoryGroup[] {
  const map = new Map<string, CommandItem[]>();
  for (const cmd of commands) {
    if (!cmd.shortcut) continue;
    const bucket = map.get(cmd.category);
    if (bucket) {
      bucket.push(cmd);
    } else {
      map.set(cmd.category, [cmd]);
    }
  }
  return Array.from(map.entries()).map(([category, items]) => ({
    category,
    items,
  }));
}
