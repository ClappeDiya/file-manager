/**
 * Non-component helpers for the Backup panel: filter-rule text format,
 * downloads and display formatting. (Folder picking: `@/lib/folder-picker`.)
 */
import type { BackupOptions, FilterRule } from "@/stores/backup-store";

export const inputCls =
  "w-full px-2 py-1 text-xs rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 text-zinc-900 dark:text-zinc-100 focus:outline-none focus:ring-1 focus:ring-blue-500";

/** Filter rules ↔ text (`+pattern` include, `-pattern` exclude; `[re]` regex, `{group}` group). */
export function rulesToText(rules: FilterRule[]): string {
  return rules
    .map((r) => {
      const sign = r.action === "include" ? "+" : "-";
      const p = r.kind === "regex" ? `[${r.pattern}]` : r.kind === "group" ? `{${r.pattern}}` : r.pattern;
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

export function formatDate(s: string | null | undefined): string {
  if (!s) return "—";
  const d = new Date(s);
  return Number.isNaN(d.getTime()) ? s : d.toLocaleString();
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
