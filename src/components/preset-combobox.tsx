/**
 * Searchable rule-template picker (Advanced). A WAI-ARIA combobox: type to
 * filter by name, description or tag; Up/Down move, Enter picks, Escape
 * closes, Home/End jump. Replaces a plain `<select>` that cannot be searched.
 */
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { searchPresets, type RulePreset } from "@/lib/automation-rule-model";

const field = "w-full min-w-0 rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";

export function PresetCombobox({ presets, onPick, className = "" }: { presets: RulePreset[]; onPick: (p: RulePreset) => void; className?: string }) {
  const listId = useId();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const matches = useMemo(() => searchPresets(presets, query), [presets, query]);
  const optionId = (i: number) => `${listId}-opt-${i}`;

  useEffect(() => setActive(0), [query]);
  // Close when focus or a click lands elsewhere.
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  const pick = (p: RulePreset | undefined) => {
    if (!p) return;
    onPick(p);
    setQuery("");
    setOpen(false);
  };
  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setOpen(true);
        if (open) setActive((a) => Math.min(a + 1, matches.length - 1));
        break;
      case "ArrowUp":
        e.preventDefault();
        setActive((a) => Math.max(a - 1, 0));
        break;
      case "Home":
        if (open) {
          e.preventDefault();
          setActive(0);
        }
        break;
      case "End":
        if (open) {
          e.preventDefault();
          setActive(Math.max(matches.length - 1, 0));
        }
        break;
      case "Enter":
        if (open) {
          e.preventDefault();
          pick(matches[active]);
        }
        break;
      case "Escape":
        if (open) {
          e.stopPropagation(); // keep the editor open
          setOpen(false);
        }
        break;
    }
  };

  return (
    <div ref={root} className={`relative ${className}`}>
      <input
        role="combobox"
        aria-label="Start from a template"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && matches[active] ? optionId(active) : undefined}
        className={field}
        placeholder="Search templates…"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
      />
      {open && (
        <ul id={listId} role="listbox" aria-label="Templates" className="absolute right-0 z-20 mt-1 max-h-64 w-72 overflow-y-auto rounded border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-lg">
          {matches.map((p, i) => (
            <li
              key={p.id}
              id={optionId(i)}
              role="option"
              aria-selected={i === active}
              className={`cursor-pointer px-2 py-1 text-xs ${i === active ? "bg-[var(--color-hover)]" : ""}`}
              onMouseEnter={() => setActive(i)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pick(p)}
            >
              <div className="font-medium">{p.label}</div>
              <div className="text-[11px] text-[var(--color-text-muted)]">{p.description}</div>
            </li>
          ))}
          {matches.length === 0 && (
            <li role="option" aria-selected="false" aria-disabled="true" className="px-2 py-1 text-xs text-[var(--color-text-muted)]">
              No template matches “{query}”.
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
