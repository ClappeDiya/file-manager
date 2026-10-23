/**
 * AI panel tabs backed by local data: **Help** answers questions from the
 * bundled documentation (written by the local model from the sections when
 * one runs, otherwise the sections themselves, always with sources), and
 * **Report** summarizes what automation rules and file operations did.
 */
import { useState } from "react";
import { BookOpen, ClipboardCopy, FileText, Loader2, Save } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";

/** `docs_help::DocHit` */
export interface DocHit {
  doc: string;
  section: string;
  excerpt: string;
  score: number;
}

/** `ai_engine::HelpAnswer` */
export interface HelpAnswer {
  answer: string;
  sources: DocHit[];
  generated_by: "model" | "docs";
}

/** `report::ActivityReport` */
export interface ActivityReport {
  days: number;
  from: string;
  to: string;
  markdown: string;
  summary: string | null;
  runs: number;
  failed_runs: number;
  operations: number;
  failed_operations: number;
}

const field = "w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";

export function HelpTab() {
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState<HelpAnswer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const ask = async () => {
    if (!q.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setAnswer(await tauriInvoke<HelpAnswer>("ai_help", { question: q }));
      setOpen(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex h-full flex-col gap-2 overflow-y-auto p-3 text-xs">
      <form
        className="flex gap-1"
        onSubmit={(e) => {
          e.preventDefault();
          void ask();
        }}
      >
        <input className={field} aria-label="Question" placeholder="How do I…? (answers come from the user guide)" value={q} onChange={(e) => setQ(e.target.value)} />
        <Button type="submit" size="sm" disabled={busy || !q.trim()}>
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Ask"}
        </Button>
      </form>
      {error && <p className="text-red-500" role="alert">{error}</p>}
      {answer && (
        <>
          <div className="whitespace-pre-wrap rounded border border-[var(--color-border)] p-2" aria-label="Answer">{answer.answer}</div>
          <p className="text-[color:var(--color-text-tertiary)]">
            {answer.generated_by === "model" ? "Written by your local AI model from these sections:" : answer.sources.length ? "From the documentation (no local model is running):" : ""}
          </p>
          <ul className="space-y-1" aria-label="Sources">
            {answer.sources.map((s, i) => (
              <li key={`${s.doc}-${s.section}`}>
                <button type="button" className="flex items-center gap-1 text-left text-[color:var(--color-primary)] hover:underline" aria-expanded={open === i} onClick={() => setOpen(open === i ? null : i)}>
                  <BookOpen className="h-3 w-3 shrink-0" /> {s.doc} — {s.section}
                </button>
                {open === i && <div className="mt-1 whitespace-pre-wrap rounded bg-[var(--color-bg)] p-2 text-[color:var(--color-text-secondary)]">{s.excerpt}</div>}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

export function ReportTab() {
  const [days, setDays] = useState(7);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<ActivityReport | null>(null);
  const [savePath, setSavePath] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const full = report ? (report.summary ? `> ${report.summary}\n\n` : "") + report.markdown : "";
  const run = async () => {
    setBusy(true);
    setError(null);
    setMsg(null);
    try {
      setReport(await tauriInvoke<ActivityReport>("ai_activity_report", { days }));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex h-full flex-col gap-2 overflow-y-auto p-3 text-xs">
      <div className="flex items-center gap-2">
        <select className={`${field} w-auto`} aria-label="Period" value={days} onChange={(e) => setDays(Number(e.target.value))}>
          <option value={1}>Last 24 hours</option>
          <option value={7}>Last 7 days</option>
          <option value={30}>Last 30 days</option>
        </select>
        <Button size="sm" onClick={() => void run()} disabled={busy}>
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" /> : <FileText className="h-3.5 w-3.5 mr-1" />} Create report
        </Button>
      </div>
      {error && <p className="text-red-500" role="alert">{error}</p>}
      {report && (
        <>
          <div className="grid grid-cols-2 gap-1" aria-label="Totals">
            <span>{report.runs} rule runs · <b className={report.failed_runs ? "text-red-500" : ""}>{report.failed_runs} failed</b></span>
            <span>{report.operations} file operations · <b className={report.failed_operations ? "text-red-500" : ""}>{report.failed_operations} not successful</b></span>
          </div>
          {report.summary && <p className="rounded border-l-2 border-[var(--color-primary)] pl-2 italic" aria-label="Summary">{report.summary}</p>}
          <pre className="whitespace-pre-wrap rounded border border-[var(--color-border)] bg-[var(--color-bg)] p-2 font-mono text-[11px]" aria-label="Report">{report.markdown}</pre>
          <div className="flex items-center gap-1">
            <Button variant="outline" size="sm" onClick={() => void navigator.clipboard.writeText(full).then(() => setMsg("Copied."), (e) => setError(errorText(e)))}>
              <ClipboardCopy className="h-3.5 w-3.5 mr-1" /> Copy
            </Button>
            <input className={field} aria-label="Save to" placeholder="/path/to/report.md" value={savePath} onChange={(e) => setSavePath(e.target.value)} />
            <Button
              variant="outline"
              size="sm"
              disabled={!savePath.trim()}
              onClick={() =>
                void tauriInvoke<void>("write_text_file_full", { path: savePath.trim(), content: full }).then(
                  () => setMsg(`Saved to ${savePath.trim()}.`),
                  (e) => setError(errorText(e)),
                )
              }
            >
              <Save className="h-3.5 w-3.5 mr-1" /> Save
            </Button>
          </div>
          {msg && <p className="text-green-600">{msg}</p>}
        </>
      )}
    </div>
  );
}
