/**
 * Run history (K122): every backup and maintenance run in a sortable,
 * paged table loaded from the backend one page at a time (view remembered
 * as "backup-runs"), filterable by job, text and date range (Today …
 * Custom); each run says what started it; the chosen run's log, report,
 * per-destination results and Open source / Open destination buttons open
 * below the table.
 */
import { useEffect, useState } from "react";
import type { ColumnDef } from "@tanstack/react-table";
import { tauriInvoke } from "@/hooks/use-tauri";
import { usePagedListing } from "@/hooks/use-paged-listing";
import { useTableView } from "@/hooks/use-table-view";
import { formatDate, inputCls, statusColor } from "@/lib/backup-helpers";
import { runDuration } from "@/lib/backup-time";
import { useBackupStore, type BackupRun, type SavedDestination } from "@/stores/backup-store";
import { DataGrid } from "../data-grid";
import { HistoryRangeChips } from "../history-range-chips";
import { ALL_TIME, rangeArgs, type HistoryRange } from "@/lib/history-range";
import { triggerLabel } from "@/lib/run-trigger";
import { canOpen, destinationTarget, localTarget, openTarget, type OpenTarget } from "@/lib/open-location";
import { Banner, Btn } from "./backup-ui";

const PREFS = "backup-runs";

/** One destination's result in a run (a replication report's `replica_runs`). */
interface ReplicaResult {
  id: string;
  name: string;
  status: string;
  started_at: string;
  finished_at: string | null;
  files: number;
  bytes: number;
  error: string | null;
}

/** The per-destination results a run's report carries: a replication's own, or those of the replications a backup ran after itself. */
export function replicaResults(report: Record<string, unknown>): ReplicaResult[] {
  const own = Array.isArray(report.replica_runs) ? (report.replica_runs as ReplicaResult[]) : [];
  const after = Array.isArray(report.replication) ? (report.replication as Record<string, unknown>[]).flatMap((r) => (Array.isArray(r.replica_runs) ? (r.replica_runs as ReplicaResult[]) : [])) : [];
  return [...own, ...after];
}

export function BackupRunHistory() {
  const jobs = useBackupStore((s) => s.jobs);
  const [jobId, setJobId] = useState("");
  const [filter, setFilter] = useState("");
  const [settled, setSettled] = useState("");
  const [open, setOpen] = useState<BackupRun | null>(null);
  const [range, setRange] = useState<HistoryRange>(ALL_TIME);
  const [library, setLibrary] = useState<SavedDestination[]>([]);
  const [openError, setOpenError] = useState<string | null>(null);
  const [view] = useTableView(PREFS);
  useEffect(() => {
    tauriInvoke<SavedDestination[]>("backup_list_destinations", undefined, []).then((l) => setLibrary(l ?? [])).catch(() => setLibrary([]));
  }, []);
  useEffect(() => {
    const t = setTimeout(() => setSettled(filter), 250);
    return () => clearTimeout(t);
  }, [filter]);
  const list = usePagedListing<BackupRun>("backup_run_history", { jobId: jobId || null, limit: 100, ...rangeArgs(range) }, { view, filter: settled, textOf: (r) => `${r.summary} ${r.operation}` });
  const jobName = (id: string) => jobs.find((j) => j.id === id)?.name ?? "deleted job";
  const reveal = async (t: OpenTarget) => setOpenError(await openTarget(t));

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
    { id: "trigger", header: "Started by", enableSorting: false, accessorFn: (r) => triggerLabel(r.trigger) },
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
        <HistoryRangeChips value={range} onChange={setRange} />
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
        emptyText={settled || range.preset !== "all" ? "No run matches the filter." : "No runs yet."}
        resetKey={`${jobId}|${settled}|${JSON.stringify(rangeArgs(range))}`}
      />
      {open && (
        <div className="border border-zinc-200 dark:border-zinc-700 rounded p-2 text-xs space-y-1" data-testid="run-details">
          <div className="font-medium">
            {jobName(open.job_id)} · {open.operation} · {formatDate(open.started_at)} · started by {triggerLabel(open.trigger).toLowerCase()}
          </div>
          {(() => {
            const job = jobs.find((j) => j.id === open.job_id);
            if (!job?.destination) return null;
            const sources = job.sources ?? [];
            const dest = destinationTarget(job.destination, library);
            const many = sources.length > 1;
            return (
              <div className="flex gap-1 flex-wrap items-center">
                {sources.map((src) => (
                  <Btn key={src} title={src} onClick={() => void reveal(localTarget(src))} disabled={!canOpen(localTarget(src))}>
                    {many ? `Open source: ${src.split(/[/\\]/).filter(Boolean).pop() ?? src}` : "Open source"}
                  </Btn>
                ))}
                <Btn title={dest.kind === "none" ? dest.reason : job.destination.type === "local" ? job.destination.path : "Opens in the connection browser"} onClick={() => void reveal(dest)} disabled={!canOpen(dest)}>
                  Open destination
                </Btn>
                {dest.kind === "none" && <span className="text-zinc-500">{dest.reason}</span>}
              </div>
            );
          })()}
          {openError && <Banner tone="error">{openError}</Banner>}
          {replicaResults(open.report).length > 0 && (
            <table className="w-full text-[11px]" data-testid="replica-results">
              <thead>
                <tr className="text-left text-zinc-500">
                  <th>Destination</th>
                  <th>Result</th>
                  <th>Started</th>
                  <th>Files</th>
                  <th>Took</th>
                </tr>
              </thead>
              <tbody>
                {replicaResults(open.report).map((r, i) => (
                  <tr key={`${r.id}-${i}`}>
                    <td>{r.name}</td>
                    <td className={statusColor(r.status === "success" ? "success" : "error")}>
                      {r.status}
                      {r.error ? ` — ${r.error}` : ""}
                    </td>
                    <td>{formatDate(r.started_at)}</td>
                    <td>{r.files}</td>
                    <td>{runDuration(r.started_at, r.finished_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
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
