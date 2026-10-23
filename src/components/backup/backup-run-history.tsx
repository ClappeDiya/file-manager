/**
 * Run history (K122): every backup and maintenance run in a sortable,
 * paged table loaded from the backend one page at a time (view remembered
 * as "backup-runs"), filterable by job and text; the chosen run's log and
 * report open below the table.
 */
import { useEffect, useState } from "react";
import type { ColumnDef } from "@tanstack/react-table";
import { tauriInvoke } from "@/hooks/use-tauri";
import { usePagedListing } from "@/hooks/use-paged-listing";
import { useTableView } from "@/hooks/use-table-view";
import { formatDate, inputCls, statusColor } from "@/lib/backup-helpers";
import { runDuration } from "@/lib/backup-time";
import { useBackupStore, type BackupRun } from "@/stores/backup-store";
import { DataGrid } from "../data-grid";
import { Banner, Btn } from "./backup-ui";

const PREFS = "backup-runs";

export function BackupRunHistory() {
  const jobs = useBackupStore((s) => s.jobs);
  const [jobId, setJobId] = useState("");
  const [filter, setFilter] = useState("");
  const [settled, setSettled] = useState("");
  const [open, setOpen] = useState<BackupRun | null>(null);
  const [view] = useTableView(PREFS);
  useEffect(() => {
    const t = setTimeout(() => setSettled(filter), 250);
    return () => clearTimeout(t);
  }, [filter]);
  const list = usePagedListing<BackupRun>("backup_run_history", { jobId: jobId || null, limit: 100 }, { view, filter: settled, textOf: (r) => `${r.summary} ${r.operation}` });
  const jobName = (id: string) => jobs.find((j) => j.id === id)?.name ?? "deleted job";

  const columns: ColumnDef<BackupRun>[] = [
    {
      id: "status",
      header: "Result",
      accessorFn: (r) => r.status,
      cell: ({ row: { original: r } }) => (
        <span className="whitespace-nowrap">
          <span className={statusColor(r.status)} aria-hidden="true">
            ●
          </span>{" "}
          {r.status}
        </span>
      ),
    },
    { id: "job_id", header: "Job", accessorFn: (r) => jobName(r.job_id) },
    { id: "operation", header: "Operation", accessorFn: (r) => r.operation },
    {
      id: "summary",
      header: "Summary",
      enableSorting: false,
      cell: ({ row: { original: r } }) => (
        <button type="button" className="text-left truncate max-w-[28rem] block hover:underline" title={r.summary} onClick={() => setOpen(open?.id === r.id ? null : r)}>
          {r.summary || "(no summary)"}
        </button>
      ),
    },
    { id: "started_at", header: "Started", accessorFn: (r) => r.started_at, cell: ({ row: { original: r } }) => formatDate(r.started_at), meta: { className: "whitespace-nowrap" } },
    {
      id: "finished_at",
      header: "Took",
      accessorFn: (r) => r.finished_at ?? "",
      cell: ({ row: { original: r } }) => runDuration(r.started_at, r.finished_at),
      meta: { className: "whitespace-nowrap text-zinc-500" },
    },
  ];

  return (
    <div className="p-3 space-y-2" data-testid="backup-history">
      <div className="flex gap-1 flex-wrap">
        <select className={`${inputCls} w-auto`} value={jobId} onChange={(e) => setJobId(e.target.value)} aria-label="Job">
          <option value="">All jobs</option>
          {jobs.map((j) => (
            <option key={j.id} value={j.id}>
              {j.name}
            </option>
          ))}
        </select>
        <input className={`${inputCls} w-48`} value={filter} placeholder="Filter…" aria-label="Filter runs" onChange={(e) => setFilter(e.target.value)} />
        <Btn onClick={() => void tauriInvoke<number>("backup_prune_history", { days: 90 }).then(list.reload)} title="Delete run history older than 90 days">
          Prune &gt; 90 days
        </Btn>
      </div>
      {list.error && <Banner tone="error">{list.error}</Banner>}
      <DataGrid<BackupRun>
        data={list.rows}
        columns={columns}
        label="Run history"
        dense
        pagination
        prefsKey={PREFS}
        server={list.server}
        getRowId={(r) => r.id}
        rowProps={(r) => ({ className: open?.id === r.id ? "bg-blue-50 dark:bg-blue-950" : undefined, "data-testid": `run-row-${r.id}` })}
        onRowActivate={(r) => setOpen(open?.id === r.id ? null : r)}
        emptyText={settled ? "No run matches the filter." : "No runs yet."}
        resetKey={`${jobId}|${settled}`}
      />
      {open && (
        <div className="border border-zinc-200 dark:border-zinc-700 rounded p-2 text-xs space-y-1" data-testid="run-details">
          <div className="font-medium">
            {jobName(open.job_id)} · {open.operation} · {formatDate(open.started_at)}
          </div>
          {open.log.length > 0 && (
            <ul className="font-mono text-[11px] max-h-40 overflow-y-auto">
              {open.log.map((l, i) => (
                <li key={i} className={l.level === "error" ? "text-red-600" : l.level === "warning" ? "text-amber-600" : ""}>
                  [{l.level}] {l.message}
                </li>
              ))}
            </ul>
          )}
          <pre className="text-[11px] max-h-40 overflow-auto bg-zinc-50 dark:bg-zinc-800 p-1 rounded">{JSON.stringify(open.report, null, 2)}</pre>
        </div>
      )}
    </div>
  );
}
