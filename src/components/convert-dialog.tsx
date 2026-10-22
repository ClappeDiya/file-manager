/**
 * Convert the selected images (resize, change format) or documents (to
 * PDF and other formats, with LibreOffice). Outputs are new files next to
 * the originals or in a chosen folder; results are listed per file.
 */
import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { errorText } from "@/lib/error-text";
import { dispatchRefresh } from "@/lib/refresh-affected";
import { convert, convertKind, useConvertDialog, type ConvertCapabilities, type ConvertOutcome, type ImageSpec } from "@/stores/convert-store";
import type { DocumentFormat } from "@/stores/automation-store";

const field = "w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";
const num = (v: string) => (v.trim() === "" ? null : Number(v));

export function ConvertDialog() {
  const paths = useConvertDialog((s) => s.paths);
  const close = useConvertDialog((s) => s.close);
  const kind = convertKind(paths);
  const [caps, setCaps] = useState<ConvertCapabilities | null>(null);
  const [spec, setSpec] = useState<ImageSpec>({ format: "jpeg", max_width: 1920, max_height: 1920, quality: 85 });
  const [to, setTo] = useState<DocumentFormat>("pdf");
  const [destDir, setDestDir] = useState("");
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<ConvertOutcome[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const containerRef = useFocusTrap<HTMLDivElement>({ active: paths.length > 0, onEscape: close });

  useEffect(() => {
    if (paths.length === 0) return;
    setResults(null);
    setError(null);
    void convert.capabilities().then(setCaps);
  }, [paths]);

  if (paths.length === 0) return null;
  const n = paths.length;
  const noOffice = kind === "document" && caps !== null && !caps.office;
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const out = kind === "image" ? await convert.images(paths, spec, destDir || null) : await convert.documents(paths, to, destDir || null);
      setResults(out);
      dispatchRefresh(out.flatMap((o) => (o.output ? [o.output] : [])));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div ref={containerRef} className="fixed inset-0 z-[60] flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-labelledby="convert-title">
      <div className="absolute inset-0 bg-black/50" aria-hidden="true" onClick={close} />
      <div className="relative z-10 w-full max-w-md rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-2xl text-xs">
        <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 id="convert-title" className="text-sm font-semibold">
            Convert {n} {kind === "image" ? "image" : "document"}{n === 1 ? "" : "s"}
          </h2>
          <Button variant="ghost" size="icon" onClick={close} aria-label="Close">
            <X className="h-4 w-4" />
          </Button>
        </div>
        <div className="space-y-3 px-4 py-3 max-h-[60vh] overflow-y-auto">
          {kind === null && <p role="alert">Select only images, or only documents, to convert.</p>}
          {kind === "image" && (
            <div className="grid grid-cols-2 gap-2">
              <label className="block">
                Format
                <select className={field} value={spec.format ?? ""} onChange={(e) => setSpec({ ...spec, format: (e.target.value || null) as ImageSpec["format"] })}>
                  <option value="">Keep the format</option>
                  {(caps?.image_formats ?? []).map((f) => (
                    <option key={f} value={f}>{f.toUpperCase()}</option>
                  ))}
                </select>
              </label>
              {spec.format === "jpeg" && (
                <label className="block">
                  Quality (1–100)
                  <input type="number" min={1} max={100} className={field} value={spec.quality ?? ""} onChange={(e) => setSpec({ ...spec, quality: num(e.target.value) })} />
                </label>
              )}
              <label className="block">
                Largest width (px)
                <input type="number" min={1} className={field} placeholder="Any" value={spec.max_width ?? ""} onChange={(e) => setSpec({ ...spec, max_width: num(e.target.value) })} />
              </label>
              <label className="block">
                Largest height (px)
                <input type="number" min={1} className={field} placeholder="Any" value={spec.max_height ?? ""} onChange={(e) => setSpec({ ...spec, max_height: num(e.target.value) })} />
              </label>
              <p className="col-span-2 text-[color:var(--color-text-tertiary)]">Images shrink to fit, keeping their proportions; they are never enlarged.</p>
            </div>
          )}
          {kind === "document" && (
            <label className="block">
              Convert to
              <select className={field} value={to} onChange={(e) => setTo(e.target.value as DocumentFormat)}>
                {(caps?.document_formats ?? ["pdf"]).map((f) => (
                  <option key={f} value={f}>{f.toUpperCase()}</option>
                ))}
              </select>
            </label>
          )}
          {noOffice && <p className="text-red-500" role="alert">LibreOffice was not found. Install it to convert documents.</p>}
          {kind && (
            <label className="block">
              Save into folder
              <input className={field} placeholder="Next to the originals" value={destDir} onChange={(e) => setDestDir(e.target.value)} />
            </label>
          )}
          {results && (
            <ul className="space-y-1.5" aria-label="Results">
              {results.map((r) => (
                <li key={r.input} className="rounded border border-[var(--color-border)] p-1.5">
                  <div className="font-mono truncate" title={r.input}>{r.input.split(/[\\/]/).pop()}</div>
                  {r.error ? <p className="text-red-500">{r.error}</p> : <p className="text-green-600 truncate" title={r.output ?? ""}>→ {r.output?.split(/[\\/]/).pop()}</p>}
                </li>
              ))}
            </ul>
          )}
          {error && <p className="text-red-500" role="alert">{error}</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <Button variant="ghost" size="sm" onClick={close}>{results ? "Done" : "Cancel"}</Button>
          {!results && (
            <Button size="sm" disabled={!kind || busy || noOffice} onClick={() => void run()}>
              {busy ? "Converting…" : "Convert"}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
