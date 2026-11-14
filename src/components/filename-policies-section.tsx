/**
 * Filename policies (Settings): which names may be written where.
 *
 * Rules are stored and enforced on the Rust side (ufop_core::policy):
 * copy, move, rename, new files, sync runs, transfers and automation all
 * refuse a blocked name with the rule's message. "Test a path" checks a
 * name against the saved rules.
 */
import { useCallback, useEffect, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { pickFolder } from "@/lib/folder-picker";

/** Mirrors `FilenameRule` (ufop-core policy.rs). */
export interface FilenameRule {
  id: string;
  name: string;
  scope: string;
  recursive: boolean;
  inherit: boolean;
  allowed_extensions: string[] | null;
  blocked_extensions: string[];
  required_pattern: string | null;
  forbidden_pattern: string | null;
  max_name_bytes: number | null;
  message: string | null;
  enabled: boolean;
}

export interface PolicyViolation {
  rule_id: string;
  rule_name: string;
  path: string;
  reason: string;
}

const newRule = (): FilenameRule => ({
  id: "",
  name: "New rule",
  scope: "",
  recursive: true,
  inherit: true,
  allowed_extensions: null,
  blocked_extensions: [],
  required_pattern: null,
  forbidden_pattern: null,
  max_name_bytes: null,
  message: null,
  enabled: true,
});

const input = "w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-gray-700 bg-transparent";
const lbl = "block text-[11px] text-gray-500 dark:text-gray-400";
const list = (v: string) => v.split(",").map((x) => x.trim().replace(/^\./, "")).filter(Boolean);
const orNull = (v: string) => (v.trim() ? v : null);

function RuleEditor({ rule, index, onChange, onRemove }: { rule: FilenameRule; index: number; onChange: (r: FilenameRule) => void; onRemove: () => void }) {
  const set = (patch: Partial<FilenameRule>) => onChange({ ...rule, ...patch });
  const n = index + 1;
  return (
    <li className="text-xs p-2 rounded border border-gray-200 dark:border-gray-700 space-y-2" aria-label={`Rule ${n}`}>
      <div className="flex items-center gap-2">
        <input type="checkbox" aria-label={`Rule ${n} enabled`} checked={rule.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
        <input aria-label={`Rule ${n} name`} className={`${input} font-medium`} value={rule.name} onChange={(e) => set({ name: e.target.value })} />
        <button type="button" aria-label={`Remove rule ${n}`} onClick={onRemove} className="text-red-500 hover:text-red-700">
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="flex gap-1 items-end">
        <label className={`${lbl} flex-1`}>
          Folder (empty = everywhere)
          <input className={input} value={rule.scope} onChange={(e) => set({ scope: e.target.value })} />
        </label>
        <button
          type="button"
          className="px-2 py-1 rounded border border-gray-200 dark:border-gray-700"
          onClick={async () => {
            const p = await pickFolder("Folder the rule covers");
            if (p) set({ scope: p });
          }}
        >
          Browse
        </button>
      </div>
      <div className="flex gap-3">
        <label className="flex items-center gap-1">
          <input type="checkbox" checked={rule.recursive} onChange={(e) => set({ recursive: e.target.checked })} /> Include subfolders
        </label>
        <label className="flex items-center gap-1" title="Off: rules for parent folders stop applying here">
          <input type="checkbox" checked={rule.inherit} onChange={(e) => set({ inherit: e.target.checked })} /> Also apply parent folders' rules
        </label>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <label className={lbl}>
          Blocked extensions
          <input className={input} placeholder="exe, bat" value={rule.blocked_extensions.join(", ")} onChange={(e) => set({ blocked_extensions: list(e.target.value) })} />
        </label>
        <label className={lbl}>
          Only these extensions (empty = any)
          <input
            className={input}
            placeholder="pdf, csv"
            value={rule.allowed_extensions?.join(", ") ?? ""}
            onChange={(e) => set({ allowed_extensions: list(e.target.value).length ? list(e.target.value) : null })}
          />
        </label>
        <label className={lbl}>
          Names must match (regex)
          <input className={`${input} font-mono`} placeholder="^\d{4}-\d{2}-\d{2}_" value={rule.required_pattern ?? ""} onChange={(e) => set({ required_pattern: orNull(e.target.value) })} />
        </label>
        <label className={lbl}>
          Names must not match (regex)
          <input className={`${input} font-mono`} placeholder="(?i)\bcopy\b" value={rule.forbidden_pattern ?? ""} onChange={(e) => set({ forbidden_pattern: orNull(e.target.value) })} />
        </label>
        <label className={lbl}>
          Max name length (bytes)
          <input
            type="number"
            min={1}
            className={input}
            value={rule.max_name_bytes ?? ""}
            onChange={(e) => set({ max_name_bytes: e.target.value ? Math.max(1, Number(e.target.value)) : null })}
          />
        </label>
        <label className={lbl}>
          Message shown when blocked
          <input className={input} value={rule.message ?? ""} onChange={(e) => set({ message: orNull(e.target.value) })} />
        </label>
      </div>
    </li>
  );
}

export function FilenamePoliciesSection() {
  const [rules, setRules] = useState<FilenameRule[]>([]);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [testPath, setTestPath] = useState("");
  const [testResult, setTestResult] = useState<string | null>(null);

  const load = useCallback(async () => {
    setRules(await tauriInvokeSafe<FilenameRule[]>("filename_policies_list", undefined, []));
    setDirty(false);
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const edit = (next: FilenameRule[]) => {
    setRules(next);
    setDirty(true);
    setSaved(false);
  };

  const save = async () => {
    setError("");
    try {
      setRules(await tauriInvoke<FilenameRule[]>("filename_policies_save", { rules }));
      setDirty(false);
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const test = async () => {
    const v = await tauriInvoke<PolicyViolation[]>("filename_policy_check", { paths: [testPath] }).catch((e) => {
      setTestResult(errorText(e));
      return null;
    });
    if (v) setTestResult(v.length ? `Blocked: ${v[0].reason} (${v[0].rule_name || v[0].rule_id})` : "Allowed");
  };

  return (
    <div className="space-y-3 pt-3">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Refuse files with the wrong type or name in a folder — enforced for copy, move, rename, new files, sync, transfers and automation.
      </p>
      {rules.length === 0 && <p className="text-xs text-gray-400">No rules. Every name is allowed.</p>}
      <ul className="space-y-2" aria-label="Filename rules">
        {rules.map((r, i) => (
          <RuleEditor
            key={r.id || `new-${i}`}
            rule={r}
            index={i}
            onChange={(next) => edit(rules.map((x, j) => (j === i ? next : x)))}
            onRemove={() => edit(rules.filter((_, j) => j !== i))}
          />
        ))}
      </ul>
      <div className="flex gap-2">
        <button type="button" className="flex items-center gap-1 text-xs px-2 py-1 rounded border border-gray-200 dark:border-gray-700" onClick={() => edit([...rules, newRule()])}>
          <Plus className="h-3.5 w-3.5" /> Add rule
        </button>
        <button type="button" disabled={!dirty} className="text-xs px-3 py-1 rounded bg-blue-500 text-white disabled:opacity-40" onClick={save}>
          Save rules
        </button>
        {saved && <span className="text-xs text-green-600 self-center">Saved</span>}
      </div>
      {error && <p className="text-xs text-red-500" role="alert">{error}</p>}
      <div className="flex gap-1 items-end">
        <label className={`${lbl} flex-1`}>
          Test a path against the saved rules
          <input className={input} placeholder="/data/finance/report.docx" value={testPath} onChange={(e) => setTestPath(e.target.value)} />
        </label>
        <button type="button" disabled={!testPath.trim()} className="text-xs px-2 py-1 rounded border border-gray-200 dark:border-gray-700 disabled:opacity-40" onClick={test}>
          Test
        </button>
      </div>
      {testResult && <p className={`text-xs ${testResult === "Allowed" ? "text-green-600" : "text-amber-600"}`} data-testid="policy-test-result">{testResult}</p>}
    </div>
  );
}
