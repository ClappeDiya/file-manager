/**
 * Editor for `validate_batch` rules: which files a delivered batch must
 * contain (member rules) and checks on their contents (not empty, JSON,
 * CSV columns/rows, SHA-256 manifest).
 */
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@ufop/ui-components";
import type { BatchRules, ContentCheck, ContentRule, MemberRule } from "@/stores/automation-store";

const field = "w-full min-w-0 rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";
const num = (v: string) => (v.trim() === "" ? null : Number(v));

const CHECKS: { type: ContentCheck["type"]; label: string }[] = [
  { type: "not_empty", label: "Not empty" },
  { type: "json", label: "Valid JSON" },
  { type: "csv", label: "CSV with columns" },
  { type: "sha256_manifest", label: "SHA-256 manifest matches" },
];

function defaultCheck(type: ContentCheck["type"]): ContentCheck {
  return type === "csv" ? { type, required_columns: [], min_rows: null, delimiter: null } : { type };
}

export function BatchRulesEditor({ rules, onChange }: { rules: BatchRules; onChange: (r: BatchRules) => void }) {
  const setMember = (i: number, m: Partial<MemberRule>) => onChange({ ...rules, members: rules.members.map((x, j) => (j === i ? { ...x, ...m } : x)) });
  const setContent = (i: number, c: Partial<ContentRule>) => onChange({ ...rules, content: rules.content.map((x, j) => (j === i ? { ...x, ...c } : x)) });
  return (
    <div className="space-y-3">
      <fieldset className="space-y-1">
        <legend className="text-[11px] font-medium text-[var(--color-text-muted)]">Files the batch must contain</legend>
        {rules.members.map((m, i) => (
          <div key={i} className="grid grid-cols-[2fr_1fr_1fr_1fr_auto] items-center gap-1" role="group" aria-label={`Member rule ${i + 1}`}>
            <input className={`${field} font-mono`} aria-label="Name pattern" value={m.pattern} placeholder="orders_*.csv" onChange={(e) => setMember(i, { pattern: e.target.value })} />
            <input className={field} aria-label="At least" type="number" min={0} value={m.min} onChange={(e) => setMember(i, { min: Number(e.target.value) })} />
            <input className={field} aria-label="At most" type="number" min={0} placeholder="Any" value={m.max ?? ""} onChange={(e) => setMember(i, { max: num(e.target.value) })} />
            <input className={field} aria-label="Smallest size (bytes)" type="number" min={0} placeholder="Any size" value={m.min_bytes ?? ""} onChange={(e) => setMember(i, { min_bytes: num(e.target.value) })} />
            <Button variant="ghost" size="icon" aria-label={`Remove member rule ${i + 1}`} onClick={() => onChange({ ...rules, members: rules.members.filter((_, j) => j !== i) })}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        ))}
        <p className="text-[11px] text-[var(--color-text-muted)]">Pattern · at least · at most · smallest size in bytes. Patterns match file names, ignoring case.</p>
        <Button variant="outline" size="sm" onClick={() => onChange({ ...rules, members: [...rules.members, { pattern: "", min: 1, max: null, min_bytes: null }] })}>
          <Plus className="h-3 w-3 mr-1" /> Add member rule
        </Button>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={rules.no_extra_files} onChange={(e) => onChange({ ...rules, no_extra_files: e.target.checked })} /> Other files make the batch invalid
        </label>
      </fieldset>
      <fieldset className="space-y-1">
        <legend className="text-[11px] font-medium text-[var(--color-text-muted)]">Content checks</legend>
        {rules.content.map((c, i) => (
          <div key={i} className="space-y-1 rounded border border-[var(--color-border)] p-1.5" role="group" aria-label={`Content check ${i + 1}`}>
            <div className="grid grid-cols-[2fr_2fr_auto] items-center gap-1">
              <input className={`${field} font-mono`} aria-label="Files to check" value={c.pattern} placeholder="*.csv" onChange={(e) => setContent(i, { pattern: e.target.value })} />
              <select className={field} aria-label="Check" value={c.check.type} onChange={(e) => setContent(i, { check: defaultCheck(e.target.value as ContentCheck["type"]) })}>
                {CHECKS.map((k) => (
                  <option key={k.type} value={k.type}>{k.label}</option>
                ))}
              </select>
              <Button variant="ghost" size="icon" aria-label={`Remove content check ${i + 1}`} onClick={() => onChange({ ...rules, content: rules.content.filter((_, j) => j !== i) })}>
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            </div>
            {c.check.type === "csv" && (() => {
              const chk = c.check;
              return (
                <div className="grid grid-cols-[3fr_1fr_1fr] gap-1">
                  <input className={field} aria-label="Required columns" placeholder="Required columns, comma-separated" value={chk.required_columns.join(", ")} onChange={(e) => setContent(i, { check: { ...chk, required_columns: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) } })} />
                  <input className={field} aria-label="Fewest rows" type="number" min={0} placeholder="Rows ≥" value={chk.min_rows ?? ""} onChange={(e) => setContent(i, { check: { ...chk, min_rows: num(e.target.value) } })} />
                  <input className={field} aria-label="Delimiter" placeholder="," maxLength={3} value={chk.delimiter ?? ""} onChange={(e) => setContent(i, { check: { ...chk, delimiter: e.target.value || null } })} />
                </div>
              );
            })()}
          </div>
        ))}
        <Button variant="outline" size="sm" onClick={() => onChange({ ...rules, content: [...rules.content, { pattern: "", check: { type: "not_empty" } }] })}>
          <Plus className="h-3 w-3 mr-1" /> Add content check
        </Button>
      </fieldset>
    </div>
  );
}
