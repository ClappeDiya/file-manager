/**
 * Editable address bar: the breadcrumb turned into a text field (click the
 * empty part of the breadcrumb, or Ctrl/Cmd+L). Accepts a local path, a
 * `conn://<id>/path` location or `<connection name>:path`; suggests the
 * folders of the typed parent (debounced, through the location provider)
 * and recently visited places. Enter goes there, Esc cancels; an address
 * that does not parse, does not exist or is not a folder shows an inline
 * error and keeps the field open.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@ufop/ui-components";
import {
  formatAddress,
  parseAddressInput,
  splitAddressForCompletion,
  type AddressConnection,
} from "@/lib/location";

/** How long typing pauses before the parent folder is listed. */
export const ADDRESS_SUGGEST_DEBOUNCE_MS = 150;
const MAX_SUGGESTIONS = 12;

export interface AddressBarProps {
  /** The pane's current folder (pane path). */
  path: string;
  connections: ReadonlyArray<AddressConnection>;
  /** Folders of a pane path (the location provider's listDirectory). */
  listFolders: (path: string) => Promise<ReadonlyArray<{ path: string; name: string; is_dir: boolean }>>;
  /** Whether a pane path is an existing folder: null = yes, else the error to show. */
  validate: (path: string) => Promise<string | null>;
  /** Recently visited pane paths (path-jump history), newest first. */
  history: ReadonlyArray<string>;
  onNavigate: (path: string) => void;
  onCancel: () => void;
}

interface Suggestion {
  /** What the field gets when it is chosen. */
  text: string;
  /** What the list shows. */
  label: string;
  kind: "folder" | "recent";
}

export function AddressBar({ path, connections, listFolders, validate, history, onNavigate, onCancel }: AddressBarProps) {
  const nameOf = useCallback((id: string) => connections.find((c) => c.id === id)?.name, [connections]);
  const [text, setText] = useState(() => formatAddress(path, nameOf));
  const [error, setError] = useState<string | null>(null);
  const [folders, setFolders] = useState<Suggestion[]>([]);
  const [highlight, setHighlight] = useState(-1);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listToken = useRef(0);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  // Folders of the typed parent, debounced.
  useEffect(() => {
    const { parent, partial } = splitAddressForCompletion(text);
    const parsed = parseAddressInput(parent, connections);
    const token = ++listToken.current;
    if (!parsed.ok) {
      setFolders([]);
      return;
    }
    const timer = setTimeout(() => {
      listFolders(parsed.path).then(
        (entries) => {
          if (token !== listToken.current) return;
          const lower = partial.toLowerCase();
          setFolders(
            entries
              .filter((e) => e.is_dir && e.name.toLowerCase().startsWith(lower))
              .slice(0, MAX_SUGGESTIONS)
              .map((e) => ({ text: formatAddress(e.path, nameOf), label: e.name, kind: "folder" as const })),
          );
        },
        () => token === listToken.current && setFolders([]),
      );
    }, ADDRESS_SUGGEST_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [text, connections, listFolders, nameOf]);

  const suggestions = useMemo<Suggestion[]>(() => {
    const t = text.trim().toLowerCase();
    const recent = history
      .map((p) => formatAddress(p, nameOf))
      .filter((a) => a.toLowerCase().includes(t) && a.toLowerCase() !== t)
      .slice(0, 5)
      .map((a) => ({ text: a, label: a, kind: "recent" as const }));
    const seen = new Set(folders.map((f) => f.text));
    return [...folders, ...recent.filter((r) => !seen.has(r.text))];
  }, [folders, history, text, nameOf]);

  const submit = useCallback(
    async (value: string) => {
      const parsed = parseAddressInput(value, connections);
      if (!parsed.ok) {
        setError(parsed.error);
        return;
      }
      setBusy(true);
      try {
        const problem = await validate(parsed.path);
        if (problem) {
          setError(problem);
          return;
        }
        onNavigate(parsed.path);
      } finally {
        setBusy(false);
      }
    },
    [connections, validate, onNavigate],
  );

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    } else if (e.key === "Enter") {
      e.preventDefault();
      const chosen = highlight >= 0 ? suggestions[highlight] : undefined;
      void submit(chosen ? chosen.text : text);
    } else if (e.key === "ArrowDown" && suggestions.length > 0) {
      e.preventDefault();
      setHighlight((h) => (h + 1) % suggestions.length);
    } else if (e.key === "ArrowUp" && suggestions.length > 0) {
      e.preventDefault();
      setHighlight((h) => (h <= 0 ? suggestions.length - 1 : h - 1));
    } else if (e.key === "Tab" && suggestions.length > 0) {
      e.preventDefault();
      const chosen = suggestions[highlight >= 0 ? highlight : 0];
      setText(chosen.kind === "folder" ? `${chosen.text}/` : chosen.text);
      setHighlight(-1);
    }
  };

  return (
    <div className="relative flex-1 min-w-0" data-testid="address-bar">
      <input
        ref={inputRef}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setError(null);
          setHighlight(-1);
        }}
        onKeyDown={onKeyDown}
        onBlur={(e) => {
          // Clicking a suggestion keeps the field (mousedown prevents blur);
          // anywhere else cancels.
          if (!e.currentTarget.parentElement?.contains(e.relatedTarget as Node | null)) onCancel();
        }}
        aria-label="Address"
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? "address-bar-error" : undefined}
        aria-autocomplete="list"
        aria-expanded={suggestions.length > 0}
        role="combobox"
        spellCheck={false}
        className={cn(
          "w-full h-6 px-2 rounded-[var(--radius-sm)] text-[length:var(--font-size-xs)] font-mono",
          "bg-[var(--color-bg-secondary)] text-[color:var(--color-text)] outline-none border",
          error ? "border-[var(--color-error)]" : "border-[var(--color-primary)]",
        )}
        data-testid="address-bar-input"
      />
      {(error || busy) && (
        <div
          id="address-bar-error"
          role={error ? "alert" : "status"}
          className={cn("absolute left-0 top-7 z-50 px-2 py-1 rounded text-[11px] shadow", error ? "bg-[var(--color-error)] text-white" : "bg-[var(--color-bg-elevated)] text-[color:var(--color-text-secondary)]")}
          data-testid="address-bar-error"
        >
          {error ?? "Checking…"}
        </div>
      )}
      {!error && suggestions.length > 0 && (
        <ul
          role="listbox"
          className="absolute left-0 right-0 top-7 z-50 max-h-64 overflow-y-auto rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)] shadow-lg py-1"
          data-testid="address-bar-suggestions"
        >
          {suggestions.map((s, i) => (
            <li
              key={`${s.kind}:${s.text}`}
              role="option"
              aria-selected={i === highlight}
              onMouseDown={(e) => {
                e.preventDefault();
                setText(s.text);
                void submit(s.text);
              }}
              className={cn(
                "px-2 py-1 text-[11px] cursor-pointer truncate",
                i === highlight ? "bg-[var(--color-hover-bg)]" : "hover:bg-[var(--color-hover-bg)]",
              )}
              title={s.text}
            >
              <span className="text-[color:var(--color-text-tertiary)] mr-1">{s.kind === "folder" ? "▸" : "↺"}</span>
              {s.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
