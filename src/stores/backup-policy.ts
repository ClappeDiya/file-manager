/**
 * Policy templates and effective settings (Kopia-parity WP6: K17/K18).
 * Wire types mirror `src-tauri/src/backup_engine/templates.rs`.
 *
 * Resolution order (decided in Rust, shown here): built-in defaults <
 * global defaults < templates (in order) < the job's own settings <
 * organization (managed) templates that are enforced.
 */
import { create } from "zustand";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import type { BackupJob, BackupOptions } from "./backup-store";

/** Job sections a template sets as a whole (`templates::SECTIONS`). */
export const SETTING_SECTIONS = ["filter", "retention", "schedule", "maintenance", "scripts", "notifications", "notify_on", "throttle", "retry", "prevent_sleep"] as const;
export type SettingSection = (typeof SETTING_SECTIONS)[number];

/** Options that belong to one job and are never inherited (`templates::PER_JOB_OPTIONS`). */
export const PER_JOB_OPTIONS = ["prefix", "block_size", "block_hash", "file_hash", "dry_run", "task_config", "changed_paths", "deleted_paths"];

/** A partial set of job settings; absent / null = not set. */
export type SettingsPatch = { options?: Partial<BackupOptions> } & { [K in SettingSection]?: BackupJob[K] | null };

export interface PolicyTemplate {
  id: string;
  name: string;
  description: string;
  settings: SettingsPatch;
  updated_at: string;
  /** Delivered by the organization's console (read-only). */
  managed: boolean;
  /** Managed only: wins over every job's own settings. */
  enforced: boolean;
}

export type SettingSource =
  | { type: "built_in" }
  | { type: "global" }
  | { type: "template"; id: string; name: string }
  | { type: "job" }
  | { type: "managed"; id: string; name: string };

export interface EffectiveSetting {
  field_path: string;
  value: unknown;
  source: SettingSource;
  /** What "reset to inherited" gives, when the job (or the console) overrides. */
  inherited: { value: unknown; source: SettingSource } | null;
}

export interface EffectiveSettings {
  job_id: string;
  template_ids: string[];
  overrides: string[] | null;
  settings: EffectiveSetting[];
  warnings: string[];
}

export function newTemplate(): PolicyTemplate {
  return { id: "", name: "", description: "", settings: {}, updated_at: "", managed: false, enforced: false };
}

export function describeSource(s: SettingSource): string {
  switch (s.type) {
    case "built_in":
      return "Default";
    case "global":
      return "Global defaults";
    case "template":
      return `Template: ${s.name}`;
    case "job":
      return "This job";
    case "managed":
      return `Organization: ${s.name}`;
  }
}

/** Every field path of a job (`templates::all_paths`). */
export function settingPaths(job: BackupJob): string[] {
  return [...Object.keys(job.options).sort().map((k) => `options.${k}`), ...SETTING_SECTIONS];
}

export function getPath(job: BackupJob, path: string): unknown {
  if (path.startsWith("options.")) return (job.options as unknown as Record<string, unknown>)[path.slice(8)];
  return (job as unknown as Record<string, unknown>)[path];
}

export function setPath(job: BackupJob, path: string, value: unknown): BackupJob {
  if (path.startsWith("options.")) return { ...job, options: { ...job.options, [path.slice(8)]: value } as BackupOptions };
  return { ...job, [path]: value } as BackupJob;
}

/** Field paths whose values differ between two versions of a job. */
export function changedPaths(before: BackupJob, after: BackupJob): string[] {
  return settingPaths(after).filter((p) => JSON.stringify(getPath(before, p) ?? null) !== JSON.stringify(getPath(after, p) ?? null));
}

/** Field paths a patch sets. */
export function patchPaths(p: SettingsPatch): string[] {
  const out = Object.keys(p.options ?? {}).map((k) => `options.${k}`);
  for (const s of SETTING_SECTIONS) if (p[s] !== undefined && p[s] !== null) out.push(s);
  return out;
}

/** A patch of the given paths, taken from a job (for "Save as template"). */
export function patchFromJob(job: BackupJob, paths: string[]): SettingsPatch {
  const p: SettingsPatch = {};
  for (const path of paths) {
    if (path.startsWith("options.")) {
      const k = path.slice(8);
      if (PER_JOB_OPTIONS.includes(k)) continue;
      p.options = { ...(p.options ?? {}), [k]: getPath(job, path) } as Partial<BackupOptions>;
    } else {
      (p as Record<string, unknown>)[path] = getPath(job, path);
    }
  }
  return p;
}

interface PolicyState {
  templates: PolicyTemplate[];
  error: string | null;
  loadTemplates: () => Promise<void>;
  saveTemplate: (t: PolicyTemplate) => Promise<PolicyTemplate>;
  deleteTemplate: (id: string) => Promise<void>;
  exportTemplates: (ids?: string[]) => Promise<string>;
  importTemplates: (json: string) => Promise<PolicyTemplate[]>;
}

export const usePolicyStore = create<PolicyState>((set, get) => ({
  templates: [],
  error: null,
  loadTemplates: async () => {
    if (!isBackendAvailable()) return;
    try {
      set({ templates: await tauriInvoke<PolicyTemplate[]>("backup_list_templates"), error: null });
    } catch (e) {
      set({ error: String((e as { message?: string })?.message ?? e) });
    }
  },
  saveTemplate: async (template) => {
    const saved = await tauriInvoke<PolicyTemplate>("backup_save_template", { template });
    await get().loadTemplates();
    return saved;
  },
  deleteTemplate: async (id) => {
    await tauriInvoke<null>("backup_delete_template", { id });
    await get().loadTemplates();
  },
  exportTemplates: (ids) => tauriInvoke<string>("backup_export_templates", { ids: ids ?? null }),
  importTemplates: async (json) => {
    const r = await tauriInvoke<PolicyTemplate[]>("backup_import_templates", { json });
    await get().loadTemplates();
    return r;
  },
}));

/** Effective settings of a saved job (`id`) or of the editor's unsaved job. */
export function fetchEffectiveSettings(arg: { id: string } | { job: BackupJob }): Promise<EffectiveSettings> {
  return tauriInvoke<EffectiveSettings>("backup_effective_settings", "id" in arg ? { id: arg.id, job: null } : { id: null, job: arg.job });
}
