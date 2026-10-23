/**
 * Policies, schedules and maintenance (Kopia-parity WP6), all Advanced:
 * - `TemplatesManager` (Settings): named policy templates, import/export.
 * - `TemplatePicker` (job editor): "Based on templates", in order.
 * - `EffectiveSettingsPanel` (job editor): each setting with a badge of
 *   where it comes from, and "Reset to inherited" for the job's overrides.
 * - `MaintenanceScheduleEditor` (job editor): compact / verify / retention
 *   on their own schedules.
 * - `VerifyOptionsEditor` and `CustomVerify` (maintenance): scope and sampling.
 */
import { useEffect, useState } from "react";
import { downloadText, inputCls } from "@/lib/backup-helpers";
import {
  errorText,
  newVerifyOptions,
  type BackupJob,
  type BackupOperation,
  type BackupSchedule,
  type MaintenanceSchedule,
  type VerifyMode,
  type VerifyOptions,
} from "@/stores/backup-store";
import {
  describeSource,
  fetchEffectiveSettings,
  newTemplate,
  patchPaths,
  usePolicyStore,
  type EffectiveSetting,
  type EffectiveSettings,
  type PolicyTemplate,
  type SettingSource,
} from "@/stores/backup-policy";
import { Banner, Btn, Check, Field, Section } from "./backup-ui";
import { ScheduleEditor } from "./backup-schedule-editor";

const badgeCls: Record<SettingSource["type"], string> = {
  built_in: "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400",
  global: "bg-sky-100 text-sky-700 dark:bg-sky-900/40 dark:text-sky-300",
  template: "bg-violet-100 text-violet-700 dark:bg-violet-900/40 dark:text-violet-300",
  job: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
  managed: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300",
};

export function SourceBadge({ source }: { source: SettingSource }) {
  return <span className={`px-1.5 py-0.5 rounded text-[10px] whitespace-nowrap ${badgeCls[source.type]}`}>{describeSource(source)}</span>;
}

function short(v: unknown): string {
  const s = JSON.stringify(v);
  return s === undefined ? "—" : s.length > 80 ? `${s.slice(0, 79)}…` : s;
}

// ── Settings → Templates ──

export function TemplatesManager() {
  const { templates, error, loadTemplates, saveTemplate, deleteTemplate, exportTemplates, importTemplates } = usePolicyStore();
  const [edit, setEdit] = useState<PolicyTemplate | null>(null);
  const [settingsText, setSettingsText] = useState("{}");
  const [msg, setMsg] = useState<{ tone: "error" | "success"; text: string } | null>(null);
  const [importText, setImportText] = useState<string | null>(null);

  useEffect(() => {
    void loadTemplates();
  }, [loadTemplates]);

  const open = (t: PolicyTemplate) => {
    setEdit(t);
    setSettingsText(JSON.stringify(t.settings, null, 2));
  };
  const run = async (fn: () => Promise<string | void>) => {
    setMsg(null);
    try {
      const text = await fn();
      if (text) setMsg({ tone: "success", text });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    }
  };

  return (
    <Section title="Policy templates" hint="Named sets of settings jobs can be based on. Order: defaults < global defaults < templates < the job's own settings < organization (enforced).">
      {(error || msg) && (
        <Banner tone={msg?.tone ?? "error"} onClose={() => setMsg(null)}>
          {msg?.text ?? error}
        </Banner>
      )}
      <ul className="text-xs divide-y divide-zinc-200 dark:divide-zinc-700" data-testid="templates-list">
        {templates.map((t) => (
          <li key={t.id} className="flex items-center gap-2 py-1">
            <span className="font-medium">{t.name}</span>
            {t.managed && <SourceBadge source={{ type: "managed", id: t.id, name: t.enforced ? "enforced" : "read-only" }} />}
            <span className="text-zinc-500 truncate flex-1">{patchPaths(t.settings).join(", ") || "no settings"}</span>
            {!t.managed && <Btn onClick={() => open(t)}>Edit</Btn>}
            {!t.managed && <Btn tone="danger" onClick={() => run(async () => { await deleteTemplate(t.id); return `Deleted ${t.name}`; })}>Delete</Btn>}
            {t.managed && <Btn onClick={() => open({ ...t, id: "", name: `${t.name} (copy)`, managed: false, enforced: false })}>Copy</Btn>}
          </li>
        ))}
        {templates.length === 0 && <li className="py-1 text-zinc-500">No templates yet.</li>}
      </ul>
      <div className="flex gap-1 flex-wrap">
        <Btn onClick={() => open(newTemplate())} testId="template-new">New template</Btn>
        <Btn onClick={() => run(async () => { downloadText("backup-templates.json", await exportTemplates()); })}>Export all</Btn>
        <Btn onClick={() => setImportText(importText === null ? "" : null)}>Import…</Btn>
      </div>
      {importText !== null && (
        <div className="space-y-1">
          <textarea className={`${inputCls} font-mono`} rows={4} placeholder="Paste a template file" value={importText} onChange={(e) => setImportText(e.target.value)} data-testid="template-import-text" />
          <Btn
            onClick={() =>
              run(async () => {
                const r = await importTemplates(importText);
                setImportText(null);
                return `Imported ${r.length} template(s)`;
              })
            }
          >
            Import
          </Btn>
        </div>
      )}
      {edit && (
        <div className="space-y-2 border border-zinc-200 dark:border-zinc-700 rounded p-2" data-testid="template-editor">
          <Field label="Name">
            <input className={inputCls} value={edit.name} onChange={(e) => setEdit({ ...edit, name: e.target.value })} data-testid="template-name" />
          </Field>
          <Field label="Description">
            <input className={inputCls} value={edit.description} onChange={(e) => setEdit({ ...edit, description: e.target.value })} />
          </Field>
          <Field
            label="Settings (JSON)"
            hint='Only what you set applies, e.g. {"retention": {"type": "keep_count", "count": 10}, "options": {"compression_level": 3}, "schedule": {"enabled": true, "cron": ["0 2 * * *"]}}'
          >
            <textarea className={`${inputCls} font-mono`} rows={8} value={settingsText} onChange={(e) => setSettingsText(e.target.value)} data-testid="template-settings" />
          </Field>
          <div className="flex gap-1">
            <Btn
              tone="primary"
              testId="template-save"
              onClick={() =>
                run(async () => {
                  let settings: PolicyTemplate["settings"];
                  try {
                    settings = JSON.parse(settingsText || "{}");
                  } catch {
                    throw new Error("The settings are not valid JSON");
                  }
                  const saved = await saveTemplate({ ...edit, settings });
                  setEdit(null);
                  return `Saved ${saved.name}`;
                })
              }
            >
              Save template
            </Btn>
            <Btn onClick={() => setEdit(null)}>Cancel</Btn>
          </div>
        </div>
      )}
    </Section>
  );
}

// ── Job editor: based on templates ──

export function TemplatePicker({ value, onChange }: { value: string[]; onChange: (ids: string[]) => void }) {
  const { templates, loadTemplates } = usePolicyStore();
  useEffect(() => {
    void loadTemplates();
  }, [loadTemplates]);
  const choosable = templates.filter((t) => !t.enforced && !value.includes(t.id));
  const name = (id: string) => templates.find((t) => t.id === id)?.name ?? `${id} (missing)`;
  const move = (i: number, d: -1 | 1) => {
    const next = [...value];
    [next[i], next[i + d]] = [next[i + d], next[i]];
    onChange(next);
  };
  const enforced = templates.filter((t) => t.enforced);
  return (
    <Field label="Based on templates" hint="Later templates win over earlier ones; the job's own settings win over all of them.">
      <ol className="text-xs space-y-1" data-testid="template-picker">
        {value.map((id, i) => (
          <li key={id} className="flex items-center gap-1">
            <span className="flex-1">
              {i + 1}. {name(id)}
            </span>
            <Btn disabled={i === 0} onClick={() => move(i, -1)} title="Earlier (weaker)">↑</Btn>
            <Btn disabled={i === value.length - 1} onClick={() => move(i, 1)} title="Later (stronger)">↓</Btn>
            <Btn onClick={() => onChange(value.filter((x) => x !== id))}>Remove</Btn>
          </li>
        ))}
      </ol>
      {choosable.length > 0 && (
        <select className={inputCls} value="" onChange={(e) => e.target.value && onChange([...value, e.target.value])} data-testid="template-add">
          <option value="">Add a template…</option>
          {choosable.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
              {t.managed ? " (organization)" : ""}
            </option>
          ))}
        </select>
      )}
      {enforced.length > 0 && <span className="block text-[11px] text-red-600">Your organization enforces: {enforced.map((t) => t.name).join(", ")}</span>}
    </Field>
  );
}

// ── Job editor: effective settings ──

export function EffectiveSettingsPanel({ job, onReset }: { job: BackupJob; onReset: (path: string, value: unknown, explicit: string[]) => void }) {
  const [data, setData] = useState<EffectiveSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [onlyOverrides, setOnlyOverrides] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => {
      fetchEffectiveSettings({ job })
        .then((d) => {
          setData(d);
          setError(null);
        })
        .catch((e) => setError(errorText(e)));
    }, 300);
    return () => clearTimeout(t);
  }, [job]);

  const rows: EffectiveSetting[] = (data?.settings ?? []).filter(
    (s) => (!filter || s.field_path.includes(filter)) && (!onlyOverrides || s.source.type !== "built_in"),
  );
  const explicit = (data?.settings ?? []).filter((s) => s.source.type === "job").map((s) => s.field_path);
  return (
    <Section title="Effective settings" hint="What this job runs with and where each setting comes from.">
      {error && <Banner tone="error">{error}</Banner>}
      {data?.warnings.map((w) => (
        <Banner key={w} tone="warning">
          {w}
        </Banner>
      ))}
      <div className="flex gap-2 items-center">
        <input className={inputCls} placeholder="Filter settings" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <Check label="Hide defaults" checked={onlyOverrides} onChange={setOnlyOverrides} />
      </div>
      <div className="max-h-72 overflow-y-auto">
        <table className="w-full text-[11px]" data-testid="effective-settings">
          <tbody>
            {rows.map((s) => (
              <tr key={s.field_path} className="border-b border-zinc-100 dark:border-zinc-800" data-testid={`effective-${s.field_path}`}>
                <td className="py-0.5 pr-2 font-mono">{s.field_path}</td>
                <td className="pr-2">
                  <SourceBadge source={s.source} />
                </td>
                <td className="pr-2 font-mono text-zinc-600 dark:text-zinc-400 break-all">{short(s.value)}</td>
                <td>
                  {s.source.type === "job" && s.inherited && !s.field_path.startsWith("options.prefix") && (
                    <Btn title={`Use ${describeSource(s.inherited.source)}: ${short(s.inherited.value)}`} onClick={() => onReset(s.field_path, s.inherited!.value, explicit)} testId={`reset-${s.field_path}`}>
                      Reset to inherited
                    </Btn>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Section>
  );
}

// ── Job editor: maintenance schedule ──

const defaultTaskSchedule = (cron: string): BackupSchedule => ({ enabled: true, start_time: "03:00", interval_minutes: 10080, weekdays: [], skip_on_battery: false, calendar_ids: [], cron: [cron], timezone: null });

export function MaintenanceScheduleEditor({ value, onChange, autoCompactDays }: { value: MaintenanceSchedule; onChange: (m: MaintenanceSchedule) => void; autoCompactDays: number | null }) {
  const set = (p: Partial<MaintenanceSchedule>) => onChange({ ...value, ...p });
  return (
    <Section title="Maintenance schedule" hint="Run compaction, verification and retention on their own schedule instead of after each backup. Each takes the repository lock it needs, shows in the history and pauses with the scheduler.">
      <Check label="Compact on its own schedule" checked={value.compact !== null} onChange={(v) => set({ compact: v ? defaultTaskSchedule("0 3 * * Sun") : null })} />
      {value.compact ? (
        <ScheduleEditor value={value.compact} onChange={(compact) => set({ compact })} label="Enabled" showBattery testId="maint-compact" />
      ) : (
        <p className="text-[11px] text-zinc-500">Compaction runs after backups{autoCompactDays ? `, at most every ${autoCompactDays} day(s)` : ""}.</p>
      )}
      <Check
        label="Verify on a schedule"
        checked={value.verify !== null}
        onChange={(v) => set({ verify: v ? { schedule: defaultTaskSchedule("0 4 * * Sat"), options: { ...newVerifyOptions(), mode: { type: "sample_percent", percent: 10 }, files_percent: 1 } } : null })}
      />
      {value.verify && (
        <>
          <ScheduleEditor value={value.verify.schedule} onChange={(schedule) => set({ verify: { ...value.verify!, schedule } })} label="Enabled" testId="maint-verify" />
          <VerifyOptionsEditor value={value.verify.options} onChange={(options) => set({ verify: { ...value.verify!, options } })} />
        </>
      )}
      <Check label="Apply retention on its own schedule" checked={value.retention !== null} onChange={(v) => set({ retention: v ? defaultTaskSchedule("30 3 * * *") : null })} />
      {value.retention && <ScheduleEditor value={value.retention} onChange={(retention) => set({ retention })} label="Enabled" testId="maint-retention" />}
    </Section>
  );
}

// ── Verification scope and sampling ──

const lines = (t: string) => t.split("\n").map((s) => s.trim()).filter(Boolean);

export function VerifyOptionsEditor({ value, onChange }: { value: VerifyOptions; onChange: (v: VerifyOptions) => void }) {
  const set = (p: Partial<VerifyOptions>) => onChange({ ...value, ...p });
  const mode = value.mode;
  const setMode = (type: VerifyMode["type"]) =>
    set({ mode: type === "sample_volumes" ? { type, count: 3 } : type === "sample_percent" ? { type, percent: 10 } : { type } });
  return (
    <div className="space-y-2" data-testid="verify-options">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <Field label="Volumes to download">
          <select className={inputCls} value={mode.type} onChange={(e) => setMode(e.target.value as VerifyMode["type"])} data-testid="verify-mode">
            <option value="structure">None — check the index and listing only</option>
            <option value="sample_volumes">A number of volumes</option>
            <option value="sample_percent">A percentage of volumes</option>
            <option value="full">All, checking every block</option>
          </select>
        </Field>
        {mode.type === "sample_volumes" && (
          <Field label="Volumes">
            <input className={inputCls} type="number" min={1} value={mode.count} onChange={(e) => set({ mode: { type: "sample_volumes", count: Math.max(1, Number(e.target.value)) } })} />
          </Field>
        )}
        {mode.type === "sample_percent" && (
          <Field label="Percent of volumes">
            <input className={inputCls} type="number" min={1} max={100} value={mode.percent} onChange={(e) => set({ mode: { type: "sample_percent", percent: Math.min(100, Math.max(1, Number(e.target.value))) } })} data-testid="verify-percent" />
          </Field>
        )}
        <Field label="Rebuild and hash-check files (%)" hint="Downloads, decrypts and decompresses that share of files and compares their hashes.">
          <input className={inputCls} type="number" min={0} max={100} value={value.files_percent ?? 0} onChange={(e) => set({ files_percent: Math.min(100, Math.max(0, Number(e.target.value))) || null })} data-testid="verify-files-percent" />
        </Field>
        <Field label="Parallel downloads (0 = automatic)">
          <input className={inputCls} type="number" min={0} max={64} value={value.parallelism} onChange={(e) => set({ parallelism: Math.max(0, Number(e.target.value)) })} />
        </Field>
        <Field label="Stop after this many problems (0 = never)">
          <input className={inputCls} type="number" min={0} value={value.max_errors} onChange={(e) => set({ max_errors: Math.max(0, Number(e.target.value)) })} />
        </Field>
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <Field label="Only these versions (one per line)" hint="0 = newest, latest, oldest, or a version's time.">
          <textarea className={`${inputCls} font-mono`} rows={2} defaultValue={value.scope.versions.join("\n")} onChange={(e) => set({ scope: { ...value.scope, versions: lines(e.target.value) } })} data-testid="verify-versions" />
        </Field>
        <Field label="Only these paths (one per line)" hint="A folder, or a wildcard like */Documents/*.">
          <textarea className={`${inputCls} font-mono`} rows={2} defaultValue={value.scope.paths.join("\n")} onChange={(e) => set({ scope: { ...value.scope, paths: lines(e.target.value) } })} data-testid="verify-paths" />
        </Field>
      </div>
      <Check label="Use the local volume cache" checked={value.use_cache} onChange={(v) => set({ use_cache: v })} hint="Off: everything is downloaded, so the destination itself is checked." />
    </div>
  );
}

/** The Advanced "Custom check" of the maintenance view. */
export function CustomVerify({ busy, onRun }: { busy: boolean; onRun: (op: BackupOperation) => void }) {
  const [opts, setOpts] = useState<VerifyOptions>({ ...newVerifyOptions(), mode: { type: "sample_percent", percent: 10 }, files_percent: 5 });
  return (
    <Section title="Custom check" hint="Choose which versions and paths to check, how much to download, and when to stop.">
      <VerifyOptionsEditor value={opts} onChange={setOpts} />
      <Btn disabled={busy} onClick={() => onRun({ type: "verify", full: false, options: opts })} testId="verify-custom-run">
        {busy ? "Working…" : "Run check"}
      </Btn>
    </Section>
  );
}
