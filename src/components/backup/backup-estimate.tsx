/**
 * Everyday run helpers in the job list: "Estimate" (what the next backup
 * reads, at most uploads and roughly how long it takes) and "Back up all
 * now" (queues every job; read-only and busy ones are skipped).
 */
import { useState } from "react";
import { formatBytes } from "@/lib/format-bytes";
import { describeEstimate, describeRunAll } from "@/lib/backup-helpers";
import { errorText, useBackupStore, type BackupEstimate } from "@/stores/backup-store";
import { Banner, Btn } from "./backup-ui";

export function EstimatePanel({ estimate: e, onClose }: { estimate: BackupEstimate; onClose: () => void }) {
  return (
    <div className="mt-2 rounded border border-zinc-200 dark:border-zinc-700 p-2 text-xs space-y-1" data-testid="backup-estimate">
      <div className="flex justify-between gap-2">
        <span className="font-medium">{describeEstimate(e)}</span>
        <button className="underline text-zinc-500 shrink-0" onClick={onClose}>Close</button>
      </div>
      <div className="text-zinc-500">
        {formatBytes(e.bytes_examined)} in {e.files} files and {e.folders} folders · {e.unchanged_files} unchanged · {e.deleted_files} deleted
        {e.upper_bound && ` · at least ${formatBytes(e.new_bytes_at_least)} (changed files may share most of their data with the last version)`}
      </div>
      {e.bytes_per_sec != null && (
        <div className="text-zinc-500">
          Speed {formatBytes(e.bytes_per_sec)}/s {e.speed_source === "history" ? "from recent backups" : "(assumed: no backup history yet)"}
        </div>
      )}
      {e.largest.length > 0 && (
        <ul className="font-mono text-[11px] max-h-28 overflow-y-auto">
          {e.largest.map((f) => (
            <li key={f.path} className="flex gap-2">
              <span className="w-16 text-right shrink-0">{formatBytes(f.size)}</span>
              <span className="w-14 shrink-0 text-zinc-500">{f.status}</span>
              <span className="truncate" title={f.path}>{f.path}</span>
            </li>
          ))}
        </ul>
      )}
      {e.warnings.map((w) => (
        <div key={w} className="text-amber-600">{w}</div>
      ))}
    </div>
  );
}

/** "Back up all now" for the job list header. */
export function RunAllButton() {
  const { runAll, jobs } = useBackupStore();
  const [msg, setMsg] = useState<{ tone: "success" | "error" | "warning"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    try {
      const r = await runAll();
      setMsg({ tone: r.skipped.length ? "warning" : "success", text: describeRunAll(r) });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  if (jobs.length === 0) return null;
  return (
    <div className="space-y-1">
      <Btn tone="primary" disabled={busy} onClick={() => void go()} testId="backup-run-all" title="Queue a backup of every job; they run one after another (or as many at once as allowed)">
        Back up all now
      </Btn>
      {msg && (
        <Banner tone={msg.tone} onClose={() => setMsg(null)}>
          {msg.text}
        </Banner>
      )}
    </div>
  );
}
