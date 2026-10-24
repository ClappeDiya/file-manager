/**
 * Ordered include/exclude rules (first match wins), shared by backup jobs
 * and sync pairs — the UI of `ufop_core::filter_rules`:
 *
 * - `FilterRulesEditor`: the rule list (action, kind, pattern, reorder, remove).
 * - `FilterRulesSummary`: the rules read-only, with remove and clear (the
 *   Standard view, where presets can add rules but the editor is hidden).
 * - `ImportRulesButton`: read an rclone `--filter-from` file (`+ pattern` /
 *   `- pattern` / `# comment`) and append its rules.
 * - `FilterPresetPicker`: apply a user-saved preset, save the current rules
 *   as one (both views), delete presets (Advanced).
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { inputCls } from "@/lib/backup-helpers";
import type { FilterGroupInfo, FilterRule } from "@/stores/backup-store";
import { useUIStore } from "@/stores/ui-store";
import { Btn } from "./backup/backup-ui";

/** Mirrors `UserFilterPreset` in src-tauri sync_engine/filter_presets.rs. */
export interface UserFilterPreset {
  id: string;
  name: string;
  rules: FilterRule[];
  created_at: string;
}

const KIND_LABELS: [FilterRule["kind"], string][] = [
  ["wildcard", "Wildcard"],
  ["exact", "Exact path"],
  ["regex", "Regex"],
  ["group", "Group"],
  ["rclone", "rclone glob"],
];

/** The groups for the "Group" kind, from the shared engine. */
export function useFilterGroups(): FilterGroupInfo[] {
  const [groups, setGroups] = useState<FilterGroupInfo[]>([]);
  useEffect(() => {
    let live = true;
    void tauriInvokeSafe<FilterGroupInfo[]>("get_filter_groups", undefined, []).then((g) => live && setGroups(Array.isArray(g) ? g : []));
    return () => {
      live = false;
    };
  }, []);
  return groups;
}

export function FilterRulesEditor({
  rules,
  onChange,
  groups,
}: {
  rules: FilterRule[];
  onChange: (rules: FilterRule[]) => void;
  groups: FilterGroupInfo[] | undefined;
}) {
  const setRules = onChange;
  return (
    <div className="space-y-1">
      {rules.map((r, i) => (
        <div key={i} className="flex gap-1 items-center">
          <select className={`${inputCls} w-24`} value={r.action} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, action: e.target.value as FilterRule["action"] } : x)))}>
            <option value="exclude">Exclude</option>
            <option value="include">Include</option>
          </select>
          <select className={`${inputCls} w-28`} value={r.kind} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, kind: e.target.value as FilterRule["kind"] } : x)))}>
            {KIND_LABELS.map(([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ))}
          </select>
          {r.kind === "group" ? (
            <select className={inputCls} value={r.pattern} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, pattern: e.target.value } : x)))}>
              <option value="">Choose group…</option>
              {groups?.map((g) => (
                <option key={g.name} value={g.name} title={g.patterns.join(" ")}>
                  {g.name} — {g.description}
                </option>
              ))}
            </select>
          ) : (
            <input className={`${inputCls} font-mono`} value={r.pattern} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, pattern: e.target.value } : x)))} />
          )}
          <Btn disabled={i === 0} onClick={() => setRules(rules.map((x, j) => (j === i - 1 ? rules[i] : j === i ? rules[i - 1] : x)))} title="Move up">↑</Btn>
          <Btn onClick={() => setRules(rules.filter((_, j) => j !== i))} title="Remove">✕</Btn>
        </div>
      ))}
      <Btn onClick={() => setRules([...rules, { action: "exclude", kind: "wildcard", pattern: "" }])}>Add rule</Btn>
    </div>
  );
}

/** The rules in order, read-only, each removable, plus "Clear all". */
export function FilterRulesSummary({ rules, onChange }: { rules: FilterRule[]; onChange: (rules: FilterRule[]) => void }) {
  if (rules.length === 0) return null;
  const kindLabel = (k: FilterRule["kind"]) => KIND_LABELS.find(([x]) => x === k)?.[1] ?? k;
  return (
    <div className="space-y-1" data-testid="filter-rules-summary">
      <ol className="space-y-0.5 text-[11px] text-zinc-600 dark:text-zinc-400">
        {rules.map((r, i) => (
          <li key={`${i}-${r.action}-${r.kind}-${r.pattern}`} className="flex items-center gap-1.5">
            <span className={r.action === "include" ? "text-green-700 dark:text-green-400" : "text-red-700 dark:text-red-400"}>
              {r.action === "include" ? "Include" : "Exclude"}
            </span>
            <span className="text-zinc-400">{kindLabel(r.kind)}</span>
            <code className="font-mono truncate">{r.pattern}</code>
            <button
              type="button"
              aria-label={`Remove rule ${r.pattern}`}
              className="ml-auto text-red-600 hover:underline"
              onClick={() => onChange(rules.filter((_, j) => j !== i))}
            >
              ✕
            </button>
          </li>
        ))}
      </ol>
      <Btn onClick={() => onChange([])}>Clear all rules</Btn>
    </div>
  );
}

/** Rules not already in `rules`, appended in order. */
export function appendRules(rules: FilterRule[], extra: FilterRule[]): FilterRule[] {
  const key = (r: FilterRule) => `${r.action}|${r.kind}|${r.pattern}`;
  const have = new Set(rules.map(key));
  return [...rules, ...extra.filter((r) => !have.has(key(r)))];
}

export function ImportRulesButton({ onImport }: { onImport: (rules: FilterRule[]) => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const read = async (file: File) => {
    setError(null);
    setNote(null);
    try {
      const rules = await tauriInvoke<FilterRule[]>("parse_filter_rules", { text: await file.text() });
      onImport(rules);
      setNote(`${rules.length} rule${rules.length === 1 ? "" : "s"} imported from ${file.name}`);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <span className="inline-flex flex-wrap items-center gap-2">
      <Btn onClick={() => input.current?.click()} title="rclone --filter-from format: + pattern, - pattern, # comment" testId="import-filter-rules">
        Import rules from file…
      </Btn>
      <input
        ref={input}
        type="file"
        accept=".txt,.filter,.rules,text/plain"
        className="hidden"
        data-testid="import-filter-rules-input"
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) void read(f);
        }}
      />
      {note && <span className="text-[11px] text-green-700 dark:text-green-400">{note}</span>}
      {error && (
        <span className="text-[11px] text-red-600 dark:text-red-400" role="alert">
          {error}
        </span>
      )}
    </span>
  );
}

export function FilterPresetPicker({ rules, onApply }: { rules: FilterRule[]; onApply: (rules: FilterRule[]) => void }) {
  const advanced = useUIStore((s) => s.appMode === "advanced");
  const [presets, setPresets] = useState<UserFilterPreset[]>([]);
  const [error, setError] = useState<string | null>(null);
  /** The name being typed for a new preset; `null` = not saving. */
  const [naming, setNaming] = useState<string | null>(null);

  const load = useCallback(async () => {
    const list = await tauriInvokeSafe<UserFilterPreset[]>("list_user_filter_presets", undefined, []);
    setPresets(Array.isArray(list) ? list : []);
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    const name = naming?.trim();
    if (!name) return;
    setError(null);
    try {
      await tauriInvoke<UserFilterPreset>("save_user_filter_preset", { id: null, name, rules });
      setNaming(null);
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const remove = async (p: UserFilterPreset) => {
    if (!confirm(`Delete the filter preset "${p.name}"?`)) return;
    setError(null);
    try {
      await tauriInvoke<boolean>("delete_user_filter_preset", { id: p.id });
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="space-y-1" data-testid="filter-preset-picker">
      <div className="flex flex-wrap items-center gap-1.5">
        <select
          aria-label="Apply a saved filter preset"
          className={`${inputCls} w-auto`}
          value=""
          onChange={(e) => {
            const p = presets.find((x) => x.id === e.target.value);
            if (p) onApply(p.rules);
          }}
        >
          <option value="">{presets.length ? "Apply saved preset…" : "No saved presets"}</option>
          {presets.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} ({p.rules.length} rule{p.rules.length === 1 ? "" : "s"})
            </option>
          ))}
        </select>
        {naming === null ? (
          <Btn disabled={rules.length === 0} onClick={() => setNaming("")} title="Save the current rules as a reusable preset">
            Save as preset…
          </Btn>
        ) : (
          <span className="inline-flex items-center gap-1">
            <input
              autoFocus
              aria-label="Preset name"
              placeholder="Preset name"
              className={`${inputCls} w-40`}
              value={naming}
              onChange={(e) => setNaming(e.target.value)}
              onKeyDown={(e) => {
                // Not a <form>: the picker sits inside other editors' forms.
                if (e.key === "Enter") {
                  e.preventDefault();
                  void save();
                } else if (e.key === "Escape") setNaming(null);
              }}
            />
            <Btn disabled={!naming.trim()} onClick={() => void save()}>
              Save
            </Btn>
            <Btn onClick={() => setNaming(null)}>Cancel</Btn>
          </span>
        )}
      </div>
      {advanced && presets.length > 0 && (
        <div className="flex flex-wrap gap-1 text-[11px] text-zinc-500">
          {presets.map((p) => (
            <span key={p.id} className="inline-flex items-center gap-1 rounded border border-zinc-200 dark:border-zinc-700 px-1.5">
              {p.name}
              <button type="button" aria-label={`Delete preset ${p.name}`} className="text-red-600 hover:underline" onClick={() => void remove(p)}>
                ✕
              </button>
            </span>
          ))}
        </div>
      )}
      {error && (
        <p className="text-[11px] text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
