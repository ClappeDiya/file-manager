/**
 * Copy the selection into several folders in one go.
 *
 * Candidates are the other pane's folder and the sidebar favorites; more
 * can be added with the folder picker. Each destination is copied (and
 * recorded for undo) separately by `copy_files_to_many`, so one bad folder
 * is reported without stopping the rest.
 */
import { useState } from "react";
import { FolderPlus, X } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { tauriInvoke } from "@/hooks/use-tauri";
import { pickFolder } from "@/lib/folder-picker";
import { dispatchRefresh } from "@/lib/refresh-affected";
import { errorText } from "@/lib/error-text";
import { useCopyToManyStore, type DestinationOutcome } from "@/stores/copy-to-many-store";

export function CopyToManyDialog() {
  const sources = useCopyToManyStore((s) => s.sources);
  const candidates = useCopyToManyStore((s) => s.candidates);
  const close = useCopyToManyStore((s) => s.close);
  const open = sources.length > 0;
  const [extra, setExtra] = useState<string[]>([]);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [outcomes, setOutcomes] = useState<DestinationOutcome[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setExtra([]);
    setChosen(new Set());
    setOutcomes(null);
    setError(null);
    close();
  };
  const containerRef = useFocusTrap<HTMLDivElement>({ active: open, onEscape: reset });
  if (!open) return null;

  const all = [...new Set([...candidates, ...extra])];
  const toggle = (p: string) =>
    setChosen((c) => {
      const n = new Set(c);
      if (n.has(p)) n.delete(p);
      else n.add(p);
      return n;
    });

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const dests = all.filter((p) => chosen.has(p));
      const out = await tauriInvoke<DestinationOutcome[]>("copy_files_to_many", { sourcePaths: sources, destDirs: dests });
      setOutcomes(out);
      dispatchRefresh(dests);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const n = sources.length;
  return (
    <div ref={containerRef} className="fixed inset-0 z-[60] flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-labelledby="copy-many-title">
      <div className="absolute inset-0 bg-black/50" aria-hidden="true" onClick={reset} />
      <div className="relative z-10 w-full max-w-md rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-2xl">
        <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 id="copy-many-title" className="text-sm font-semibold">
            Copy {n} item{n === 1 ? "" : "s"} to several folders
          </h2>
          <Button variant="ghost" size="icon" onClick={reset} aria-label="Close">
            <X className="h-4 w-4" />
          </Button>
        </div>
        <div className="max-h-[50vh] space-y-1 overflow-y-auto px-4 py-3 text-xs">
          {all.length === 0 && <p className="text-[var(--color-text-muted)]">Add the folders to copy into.</p>}
          {all.map((p) => {
            const o = outcomes?.find((x) => x.dest_dir === p);
            return (
              <label key={p} className="flex items-center gap-2 py-0.5">
                <input type="checkbox" checked={chosen.has(p)} disabled={!!outcomes} onChange={() => toggle(p)} />
                <span className="flex-1 truncate font-mono" title={p}>{p}</span>
                {o && (o.error ? <span className="text-red-500" title={o.error}>Failed</span> : <span className="text-green-600">Copied</span>)}
              </label>
            );
          })}
          {outcomes?.filter((o) => o.error).map((o) => (
            <p key={o.dest_dir} className="text-red-500" role="alert">{o.dest_dir}: {o.error}</p>
          ))}
          {error && <p className="text-red-500" role="alert">{error}</p>}
        </div>
        <div className="flex items-center justify-between border-t border-[var(--color-border)] px-4 py-3">
          <Button
            variant="outline"
            size="sm"
            disabled={!!outcomes}
            onClick={async () => {
              const p = await pickFolder("Add destination folder");
              if (p) {
                setExtra((x) => [...x, p]);
                setChosen((c) => new Set(c).add(p));
              }
            }}
          >
            <FolderPlus className="h-3.5 w-3.5 mr-1" /> Add folder…
          </Button>
          {outcomes ? (
            <Button size="sm" onClick={reset}>Done</Button>
          ) : (
            <Button size="sm" disabled={busy || chosen.size === 0} onClick={run}>
              {busy ? "Copying…" : `Copy to ${chosen.size} folder${chosen.size === 1 ? "" : "s"}`}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
