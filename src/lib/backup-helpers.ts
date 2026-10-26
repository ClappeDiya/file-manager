/**
 * Non-component helpers for the Backup panel: filter-rule text format,
 * downloads and display formatting. (Folder picking: `@/lib/folder-picker`.)
 */
import type { BackupEstimate, BackupOptions, BackupSchedule, FilterRule, RunAllReport } from "@/stores/backup-store";
import { formatBytes } from "@/lib/format-bytes";
import { formatBackupTime } from "@/lib/backup-time";

export const inputCls =
  "w-full px-2 py-1 text-xs rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 text-zinc-900 dark:text-zinc-100 focus:outline-none focus:ring-1 focus:ring-blue-500";

/** Filter rules ↔ text (`+pattern` include, `-pattern` exclude; `[re]` regex, `{group}` group, `rclone:glob` rclone glob). */
export function rulesToText(rules: FilterRule[]): string {
  return rules
    .map((r) => {
      const sign = r.action === "include" ? "+" : "-";
      const p = r.kind === "regex" ? `[${r.pattern}]` : r.kind === "group" ? `{${r.pattern}}` : r.kind === "rclone" ? `rclone:${r.pattern}` : r.pattern;
      return `${sign}${p}`;
    })
    .join("\n");
}

export function textToRules(text: string): FilterRule[] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 1 && (l[0] === "+" || l[0] === "-"))
    .map((l) => {
      const action = l[0] === "+" ? "include" : "exclude";
      const body = l.slice(1).trim();
      if (body.startsWith("[") && body.endsWith("]")) return { action, kind: "regex", pattern: body.slice(1, -1) } as FilterRule;
      if (body.startsWith("{") && body.endsWith("}")) return { action, kind: "group", pattern: body.slice(1, -1) } as FilterRule;
      if (body.startsWith("rclone:")) return { action, kind: "rclone", pattern: body.slice(7).trim() } as FilterRule;
      return { action, kind: /[*?]/.test(body) ? "wildcard" : "exact", pattern: body } as FilterRule;
    });
}


/** Save text as a file (browser download; works in the Tauri webview too). */
export function downloadText(name: string, text: string, mime = "application/json") {
  const url = URL.createObjectURL(new Blob([text], { type: mime }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function statusColor(status: string | null | undefined): string {
  switch (status) {
    case "success":
      return "text-green-600 dark:text-green-400";
    case "warning":
      return "text-amber-600 dark:text-amber-400";
    case "error":
      return "text-red-600 dark:text-red-400";
    case "cancelled":
      return "text-zinc-500";
    default:
      return "text-zinc-400";
  }
}

/** Backup timestamps, in local time or UTC as set in Settings → Appearance. */
export function formatDate(s: string | null | undefined): string {
  return formatBackupTime(s);
}

/** Options that are per job by nature (the engine rejects global defaults for them). */
const PER_JOB = new Set(["prefix", "block_size", "block_hash", "file_hash", "dry_run", "task_config", "changed_paths", "deleted_paths"]);

/** Only the fields that differ from the built-in defaults become global defaults. */
export function diffOptions(builtin: BackupOptions, edited: BackupOptions): Partial<BackupOptions> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(edited)) {
    if (PER_JOB.has(k)) continue;
    if (JSON.stringify(v) !== JSON.stringify((builtin as unknown as Record<string, unknown>)[k])) out[k] = v;
  }
  return out as Partial<BackupOptions>;
}

// ── Estimate and run-all summaries ──

export function formatEta(secs: number): string {
  if (secs < 60) return `${secs} s`;
  if (secs < 3600) return `${Math.floor(secs / 60)} min ${secs % 60} s`;
  const h = Math.floor(secs / 3600);
  return `${h} h ${Math.floor((secs % 3600) / 60)} min`;
}

/** One-line summary of an estimate, e.g. "3 new, 1 changed: at most 1.2 GB, about 4 min". */
export function describeEstimate(e: BackupEstimate): string {
  const data = e.upper_bound ? `at most ${formatBytes(e.new_bytes_at_most)}` : formatBytes(e.new_bytes_at_most);
  const time = e.estimated_duration_secs != null ? `, about ${formatEta(e.estimated_duration_secs)}` : "";
  if (e.new_files === 0 && e.changed_files === 0) return `No new or changed files (${e.files} checked)${e.deleted_files ? `; ${e.deleted_files} deleted` : ""}`;
  return `${e.new_files} new, ${e.changed_files} changed of ${e.files} files: ${data} to back up${time}`;
}

export function describeRunAll(r: RunAllReport): string {
  const parts = [`${r.started.length} backup(s) queued`];
  if (r.skipped.length) parts.push(`skipped ${r.skipped.map((s) => `${s.job_name || s.job_id} (${s.reason})`).join(", ")}`);
  return parts.join("; ");
}

/** Whether a schedule is in cron mode (`BackupSchedule::is_cron`). */
export function isCronSchedule(s: BackupSchedule): boolean {
  return (s.cron ?? []).some((c) => c.trim() !== "");
}

/** Cron lines as typed in the editor → the schedule's `cron` list. */
export function cronLines(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
}
