/**
 * Table view for CSV/TSV and collapsible tree for JSON in the preview
 * pane, with a toggle back to the raw text.
 */
import { useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { detectDelimiter, parseDelimited, parseJsonish, type StructuredKind } from "@/lib/structured-preview";

const MAX_ROWS = 1000;

function CsvTable({ text, fileName }: { text: string; fileName: string }) {
  const rows = useMemo(() => parseDelimited(text, detectDelimiter(fileName, text), MAX_ROWS + 1), [text, fileName]);
  if (rows.length === 0) return <p className="p-3 text-xs text-text-tertiary">Empty file</p>;
  const [header, ...body] = rows;
  const width = Math.max(...rows.map((r) => r.length));
  return (
    <div className="overflow-auto max-h-[70vh]">
      <table className="text-xs font-mono border-collapse" aria-label={`${fileName} table`}>
        <thead className="sticky top-0 bg-surface-secondary">
          <tr>
            <th className="px-2 py-1 text-right text-text-tertiary">#</th>
            {Array.from({ length: width }, (_, i) => (
              <th key={i} scope="col" className="px-2 py-1 text-left font-semibold border-b border-border-primary whitespace-nowrap">{header[i] ?? ""}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.slice(0, MAX_ROWS - 1).map((r, i) => (
            <tr key={i} className="odd:bg-surface-secondary/40">
              <td className="px-2 py-0.5 text-right text-text-tertiary select-none">{i + 1}</td>
              {Array.from({ length: width }, (_, j) => (
                <td key={j} className="px-2 py-0.5 whitespace-nowrap max-w-[24rem] truncate" title={r[j]}>{r[j] ?? ""}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="px-2 py-1 text-[11px] text-text-tertiary">
        {body.length} row{body.length === 1 ? "" : "s"} × {width} column{width === 1 ? "" : "s"} shown (preview of the first lines)
      </p>
    </div>
  );
}

function JsonNode({ name, value, depth }: { name: string | null; value: unknown; depth: number }) {
  const [open, setOpen] = useState(depth < 2);
  const isObj = value !== null && typeof value === "object";
  const label = name !== null ? <span className="text-accent-primary">{name}: </span> : null;
  if (!isObj) {
    const text = typeof value === "string" ? JSON.stringify(value) : String(value);
    const color = typeof value === "string" ? "text-green-600" : typeof value === "number" ? "text-blue-500" : "text-purple-500";
    return (
      <div className="pl-4 whitespace-pre-wrap break-all">
        {label}
        <span className={color}>{text}</span>
      </div>
    );
  }
  const entries = Array.isArray(value) ? value.map((v, i) => [String(i), v] as const) : Object.entries(value as Record<string, unknown>);
  const summary = Array.isArray(value) ? `[${entries.length}]` : `{${entries.length}}`;
  return (
    <div className={depth === 0 ? "" : "pl-4"}>
      <button type="button" className="inline-flex items-center gap-0.5 hover:underline" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        {label}
        <span className="text-text-tertiary">{summary}</span>
      </button>
      {open && entries.slice(0, 500).map(([k, v]) => <JsonNode key={k} name={k} value={v} depth={depth + 1} />)}
      {open && entries.length > 500 && <div className="pl-4 text-text-tertiary">… {entries.length - 500} more</div>}
    </div>
  );
}

function JsonTree({ text, path }: { text: string; path: string }) {
  const [full, setFull] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const parsed = useMemo(() => parseJsonish(full ?? text), [full, text]);
  useEffect(() => {
    setFull(null);
    setFailed(false);
  }, [path]);
  useEffect(() => {
    // The preview text is capped; fetch the whole file when it didn't parse.
    if (parsed || full !== null || failed) return;
    tauriInvoke<string>("read_text_file_full", { path }).then(setFull, () => setFailed(true));
  }, [parsed, full, failed, path]);
  if (!parsed) {
    return <p className="p-3 text-xs text-text-tertiary">{failed || full !== null ? "No tree: the file isn't valid JSON, or it is over 1 MB. Switch to Raw to read it." : "Loading…"}</p>;
  }
  return (
    <div className="p-3 text-xs font-mono overflow-auto max-h-[70vh]" role="tree" aria-label="JSON tree">
      <JsonNode name={null} value={parsed.value} depth={0} />
    </div>
  );
}

/** Structured view with a Table/Tree ⇄ Raw toggle; `raw` renders the text view. */
export function StructuredPreview({ kind, text, fileName, path, raw }: { kind: Exclude<StructuredKind, null>; text: string; fileName: string; path: string; raw: React.ReactNode }) {
  const [showRaw, setShowRaw] = useState(false);
  const label = kind === "csv" ? "Table" : "Tree";
  return (
    <div className="p-2">
      <div className="flex justify-end gap-1 mb-1" role="group" aria-label="Preview mode">
        {[label, "Raw"].map((m) => (
          <button
            key={m}
            type="button"
            aria-pressed={(m === "Raw") === showRaw}
            onClick={() => setShowRaw(m === "Raw")}
            className={`px-2 py-0.5 text-[11px] rounded border ${(m === "Raw") === showRaw ? "bg-accent-primary text-white border-accent-primary" : "border-border-primary"}`}
          >
            {m}
          </button>
        ))}
      </div>
      {showRaw ? raw : kind === "csv" ? <CsvTable text={text} fileName={fileName} /> : <JsonTree text={text} path={path} />}
    </div>
  );
}
