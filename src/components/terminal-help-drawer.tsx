/**
 * Command help for the terminal: a searchable side drawer listing the `ufop`
 * commands and, when rclone resolves, rclone's, each with a one-line
 * description. Clicking a command types it at the prompt (without running
 * it); the arrow shows a command's synopsis and options.
 */
import { useEffect, useMemo, useState } from "react";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { entriesOf, insertText, NO_HELP, searchEntries, type HelpEntry, type TerminalHelp } from "@/lib/terminal-help";

const MAX_SHOWN = 300;

export function TerminalHelpDrawer({ onInsert, canInsert, onClose }: { onInsert: (text: string) => void; canInsert: boolean; onClose: () => void }) {
  const [help, setHelp] = useState<TerminalHelp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    // Outside the app there is no backend to ask: an empty list. Inside, a failure is shown, not hidden.
    tauriInvoke<TerminalHelp>("terminal_help_commands", undefined, isBackendAvailable() ? undefined : NO_HELP)
      .then((h) => live && setHelp(h))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, []);

  const entries = useMemo(() => (help ? entriesOf(help) : []), [help]);
  const shown = useMemo(() => searchEntries(entries, query), [entries, query]);
  const rows = shown.slice(0, MAX_SHOWN);

  const section = (tool: HelpEntry["tool"], title: string) => {
    const mine = rows.filter((e) => e.tool === tool);
    if (mine.length === 0) return null;
    return (
      <section aria-label={title} key={tool}>
        <h3 className="px-2 pt-2 pb-1 text-[10px] uppercase tracking-wide text-[color:var(--color-text-tertiary)]">{title}</h3>
        <ul>
          {mine.map((e) => (
            <li key={e.command} className="border-b border-[var(--color-border)]">
              <div className="flex items-start gap-1 px-2 py-1">
                <button
                  type="button"
                  className="mt-0.5 w-4 text-[10px] text-[color:var(--color-text-tertiary)]"
                  aria-expanded={open === e.command}
                  aria-label={`Details of ${e.command}`}
                  disabled={!e.usage}
                  onClick={() => setOpen(open === e.command ? null : e.command)}
                >
                  {e.usage ? (open === e.command ? "▾" : "▸") : ""}
                </button>
                <button
                  type="button"
                  className="flex-1 text-left disabled:opacity-60 hover:bg-[var(--color-hover-bg)] rounded-sm px-1"
                  disabled={!canInsert}
                  title={canInsert ? `Type "${e.command}" at the prompt` : "Open a terminal session first"}
                  onClick={() => onInsert(insertText(e))}
                >
                  <div className="font-mono text-[11px] text-[color:var(--color-text)]">{e.command}</div>
                  <div className="text-[11px] text-[color:var(--color-text-secondary)]">{e.about}</div>
                </button>
              </div>
              {open === e.command && e.usage && (
                <div className="px-7 pb-2 text-[11px] space-y-1" data-testid="help-details">
                  <div className="font-mono text-[color:var(--color-text)]">{e.usage}</div>
                  {e.options.length > 0 && (
                    <ul className="space-y-0.5">
                      {e.options.map((o) => (
                        <li key={o.flags.join(",")}>
                          <span className="font-mono">{o.flags.join(", ")}</span>
                          {o.takes_value ? <span className="font-mono text-[color:var(--color-text-tertiary)]"> &lt;value&gt;</span> : null}
                          {o.help ? <span className="text-[color:var(--color-text-secondary)]"> — {o.help}</span> : null}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
            </li>
          ))}
        </ul>
      </section>
    );
  };

  return (
    <aside className="w-80 shrink-0 border-l border-[var(--color-border)] bg-[var(--color-bg-secondary)] flex flex-col" role="complementary" aria-label="Command help" data-testid="terminal-help">
      <div className="flex items-center gap-1 px-2 py-1 border-b border-[var(--color-border)]">
        <input
          type="search"
          className="flex-1 text-xs px-2 py-1 rounded border border-[var(--color-border)] bg-[var(--color-bg)]"
          placeholder="Search commands…"
          aria-label="Search commands"
          value={query}
          autoFocus
          onChange={(e) => setQuery(e.target.value)}
        />
        <button type="button" className="text-xs px-1.5 py-1 rounded hover:bg-[var(--color-hover-bg)]" aria-label="Close command help" onClick={onClose}>
          ✕
        </button>
      </div>
      <div className="flex-1 overflow-y-auto">
        {error && (
          <div className="p-2 text-xs text-red-500" role="alert">
            {error}
          </div>
        )}
        {!help && !error && <div className="p-2 text-xs text-[color:var(--color-text-secondary)]">Loading…</div>}
        {help && (
          <>
            {section("ufop", "ufop commands")}
            {help.rclone === null && help.rclone_note && (
              <p className="p-2 text-[11px] text-[color:var(--color-text-secondary)]" data-testid="rclone-note">
                {help.rclone_note}
              </p>
            )}
            {section("rclone", help.rclone_version ? `rclone commands (${help.rclone_version})` : "rclone commands")}
            {help.rclone !== null && help.rclone_note && <p className="p-2 text-[11px] text-[color:var(--color-text-secondary)]">{help.rclone_note}</p>}
            {rows.length === 0 && entries.length > 0 && <p className="p-2 text-xs text-[color:var(--color-text-secondary)]">No command matches “{query}”.</p>}
            {shown.length > rows.length && <p className="p-2 text-[11px] text-[color:var(--color-text-secondary)]">Showing the first {MAX_SHOWN} of {shown.length}; type to narrow the list.</p>}
          </>
        )}
      </div>
    </aside>
  );
}
