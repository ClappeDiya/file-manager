/**
 * Backup Panel — versioned, deduplicated, encrypted backups.
 *
 * - Dashboard of jobs with last result, versions, sizes and next run
 * - Live task queue with progress and cancel; scheduler pause/resume
 * - Setup wizard / editor, versions & restore, maintenance
 * - Run history with logs; configuration import; direct restore
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import {
  describeRetention,
  describeSchedule,
  errorText,
  newBackupJob,
  useBackupStore,
  type BackupJob,
  type DesktopSettings,
  type ImportPreview,
} from "@/stores/backup-store";
import { BackupJobEditor } from "./backup-job-editor";
import { BackupMaintenance } from "./backup-maintenance";
import { BackupRestoreView, DirectRestore } from "./backup-restore-view";
import { BackupRecoveryView } from "./backup-recovery-view";
import { BackupSettingsView } from "./backup-settings-view";
import { formatDate, inputCls, statusColor } from "@/lib/backup-helpers";
import { Banner, Btn, Section } from "./backup-ui";
import { RunAllButton } from "./backup-estimate";
import { useEstimate } from "./use-estimate";

type View =
  | { kind: "jobs" }
  | { kind: "history" }
  | { kind: "direct" }
  | { kind: "import" }
  | { kind: "settings" }
  | { kind: "recovery" }
  | { kind: "edit"; job: BackupJob }
  | { kind: "restore"; job: BackupJob }
  | { kind: "maintenance"; job: BackupJob };

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
}

function TaskQueue() {
  const { status, cancel } = useBackupStore();
  if (status.tasks.length === 0) return null;
  return (
    <div className="p-2 space-y-1 border-b border-zinc-200 dark:border-zinc-700" data-testid="backup-tasks">
      {status.tasks.map((t) => {
        const p = t.progress;
        const pct = p && p.files_total > 0 ? Math.round((p.files_processed / p.files_total) * 100) : null;
        return (
          <div key={t.task_id} className="text-xs">
            <div className="flex justify-between gap-2">
              <span className="truncate">
                <b>{t.job_name}</b> · {t.operation} · {t.state === "queued" ? "queued" : (p?.phase ?? "starting")}
                {pct !== null && ` ${pct}%`}
                {t.state === "running" && t.bytes_per_sec > 0 && ` · ${formatBytes(t.bytes_per_sec)}/s`}
                {t.state === "running" && t.eta_secs != null && ` · ${formatDuration(t.eta_secs)} left`}
              </span>
              <button className="underline text-red-600 shrink-0" onClick={() => void cancel(t.task_id)}>
                Cancel
              </button>
            </div>
            {t.state === "running" && (
              <div className="h-1 bg-zinc-200 dark:bg-zinc-700 rounded mt-0.5 overflow-hidden">
                <div className="h-full bg-blue-500 transition-all" style={{ width: `${pct ?? 5}%` }} />
              </div>
            )}
            {p?.current_path && <div className="truncate text-[11px] text-zinc-500 font-mono">{p.current_path}</div>}
          </div>
        );
      })}
    </div>
  );
}

function JobCard({ job, onView }: { job: BackupJob; onView: (v: View) => void }) {
  const { runJob, status } = useBackupStore();
  const [err, setErr] = useState<string | null>(null);
  const busy = status.tasks.some((t) => t.job_id === job.id);
  const next = status.next_runs.find((n) => n.job_id === job.id);
  const est = useEstimate(job.id);
  const start = async (dryRun: boolean) => {
    setErr(null);
    try {
      await runJob(job.id, dryRun);
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3" data-testid="backup-job-card">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="text-sm font-medium text-zinc-900 dark:text-zinc-100 truncate">
            <span className={statusColor(job.stats.last_result)} title={job.stats.last_result ?? "never run"}>●</span> {job.name}
            {job.encrypted && <span className="ml-1 text-[10px] text-zinc-500" title="Encrypted">🔒</span>}
            {job.legal_hold && (
              <span className="ml-1 text-[10px] px-1 rounded bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300" title={`Legal hold: ${job.legal_hold.matter}`} data-testid="legal-hold-badge">
                Legal hold
              </span>
            )}
            {job.read_only && (
              <span className="ml-1 text-[10px] px-1 rounded bg-sky-100 text-sky-800 dark:bg-sky-900/40 dark:text-sky-300" title="Read-only: restore and browse only" data-testid="read-only-badge">
                Read-only
              </span>
            )}
          </div>
          {job.description && <div className="text-xs text-zinc-500 truncate">{job.description}</div>}
        </div>
        <span className="text-xs text-zinc-500 shrink-0">{job.stats.version_count} version(s)</span>
      </div>
      <div className="text-xs text-zinc-500 dark:text-zinc-400 mt-1 grid grid-cols-1 sm:grid-cols-2 gap-x-3">
        <div className="truncate" title={job.sources.join("\n")}>From: {job.sources.join(", ")}</div>
        <div className="truncate">To:{" "}
          {job.destination.type === "local"
            ? job.destination.path
            : job.destination.type === "library"
              ? `library destination${job.destination.remote_path ? ` / ${job.destination.remote_path}` : ""}`
              : job.destination.type === "cloud"
                ? `${job.destination.config.provider} cloud destination`
                : `connection ${job.destination.remote_path}`}</div>
        <div>Last run: {formatDate(job.stats.last_run_at)}</div>
        <div>Next: {next ? formatDate(next.next_run_at) : describeSchedule(job.schedule)}</div>
        <div>Source: {formatBytes(job.stats.source_size)} · Stored: {formatBytes(job.stats.backend_size)}</div>
        <div className="truncate">{describeRetention(job.retention)}</div>
      </div>
      {job.stats.last_error && <div className="text-xs text-red-600 mt-1 truncate" title={job.stats.last_error}>{job.stats.last_error}</div>}
      {err && <div className="text-xs text-red-600 mt-1">{err}</div>}
      <div className="flex gap-1 mt-2 flex-wrap">
        <Btn tone="primary" disabled={busy || job.read_only} title={job.read_only ? "Read-only jobs never back up" : undefined} onClick={() => void start(false)} testId="backup-run-now">Run now</Btn>
        <Btn disabled={busy} onClick={() => void start(true)} title="Show what would be uploaded without changing anything">Dry run</Btn>
        {!job.read_only && est.button}
        <Btn onClick={() => onView({ kind: "restore", job })} testId="backup-open-restore">Restore…</Btn>
        <Btn onClick={() => onView({ kind: "edit", job })}>Edit</Btn>
        <Btn onClick={() => onView({ kind: "maintenance", job })}>Maintenance</Btn>
      </div>
      {est.panel}
    </div>
  );
}

function History() {
  const { history, loadHistory, jobs } = useBackupStore();
  const [jobId, setJobId] = useState<string>("");
  const [open, setOpen] = useState<string | null>(null);
  useEffect(() => {
    void loadHistory(jobId || null, 100);
  }, [jobId, loadHistory]);
  return (
    <div className="p-3 space-y-2" data-testid="backup-history">
      <div className="flex gap-1">
        <select className={inputCls} value={jobId} onChange={(e) => setJobId(e.target.value)}>
          <option value="">All jobs</option>
          {jobs.map((j) => (
            <option key={j.id} value={j.id}>{j.name}</option>
          ))}
        </select>
        <Btn onClick={() => void tauriInvoke<number>("backup_prune_history", { days: 90 }).then(() => loadHistory(jobId || null, 100))} title="Delete run history older than 90 days">
          Prune &gt; 90 days
        </Btn>
      </div>
      {history.length === 0 && <div className="text-sm text-zinc-500 text-center py-6">No runs yet.</div>}
      {history.map((r) => (
        <div key={r.id} className="border border-zinc-200 dark:border-zinc-700 rounded p-2 text-xs">
          <button className="w-full text-left flex justify-between gap-2" onClick={() => setOpen(open === r.id ? null : r.id)}>
            <span className="truncate">
              <span className={statusColor(r.status)}>●</span> {jobs.find((j) => j.id === r.job_id)?.name ?? "deleted job"} · {r.operation} · {r.summary}
            </span>
            <span className="text-zinc-500 shrink-0">{formatDate(r.started_at)}</span>
          </button>
          {open === r.id && (
            <div className="mt-1 space-y-1">
              {r.log.length > 0 && (
                <ul className="font-mono text-[11px] max-h-40 overflow-y-auto">
                  {r.log.map((l, i) => (
                    <li key={i} className={l.level === "error" ? "text-red-600" : l.level === "warning" ? "text-amber-600" : ""}>
                      [{l.level}] {l.message}
                    </li>
                  ))}
                </ul>
              )}
              <pre className="text-[11px] max-h-40 overflow-auto bg-zinc-50 dark:bg-zinc-800 p-1 rounded">{JSON.stringify(r.report, null, 2)}</pre>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function ImportConfig({ onDone }: { onDone: (job: BackupJob | null) => void }) {
  const [json, setJson] = useState("");
  const [pass, setPass] = useState("");
  const [encrypted, setEncrypted] = useState(false);
  const [stats, setStats] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!json) return;
    tauriInvoke<boolean>("backup_export_is_encrypted", { json }, false).then(setEncrypted).catch(() => setEncrypted(false));
  }, [json]);
  const go = async (save: boolean) => {
    setError(null);
    try {
      const r = await tauriInvoke<ImportPreview>("backup_import_config", { json, exportPassphrase: pass || null, importStats: stats, save });
      if (save) await useBackupStore.getState().loadJobs();
      onDone(r.job);
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div className="p-3 space-y-3" data-testid="backup-import">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      <Section title="Import a backup configuration" hint="From an export file or a configuration read from a backup.">
        <input
          ref={fileRef}
          type="file"
          accept=".json,application/json"
          className="text-xs"
          onChange={async (e) => {
            const f = e.target.files?.[0];
            if (f) setJson(await f.text());
          }}
        />
        <textarea className={`${inputCls} font-mono`} rows={6} value={json} onChange={(e) => setJson(e.target.value)} placeholder="…or paste JSON" />
        {encrypted && <input className={inputCls} type="password" placeholder="Export passphrase" value={pass} onChange={(e) => setPass(e.target.value)} />}
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={stats} onChange={(e) => setStats(e.target.checked)} /> Import statistics (sizes, version count)
        </label>
        <div className="flex gap-1">
          <Btn disabled={!json} onClick={() => void go(false)}>Review in editor</Btn>
          <Btn tone="success" disabled={!json} onClick={() => void go(true)}>Import and save</Btn>
          <Btn onClick={() => onDone(null)}>Cancel</Btn>
        </div>
      </Section>
    </div>
  );
}

/** Tray / start-at-login settings (desktop app only). */
function DesktopToggles() {
  const [s, setS] = useState<DesktopSettings | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    tauriInvoke<DesktopSettings>("backup_get_desktop_settings").then(setS).catch((e) => setErr(errorText(e)));
  }, []);
  if (!s) return null;
  const update = (next: DesktopSettings) =>
    tauriInvoke<DesktopSettings>("backup_set_desktop_settings", { settings: next })
      .then(setS)
      .catch((e) => setErr(errorText(e)));
  return (
    <span className="flex items-center gap-3" data-testid="backup-desktop-settings">
      <label className="flex items-center gap-1" title="Closing the window keeps UFOP in the system tray so scheduled backups run">
        <input type="checkbox" checked={s.run_in_background} onChange={(e) => void update({ ...s, run_in_background: e.target.checked })} />
        Keep running in tray
      </label>
      <label className="flex items-center gap-1" title="Start UFOP hidden in the tray when you log in">
        <input type="checkbox" checked={s.start_at_login} onChange={(e) => void update({ ...s, start_at_login: e.target.checked })} />
        Start at login
      </label>
      {err && <span className="text-red-600">{err}</span>}
    </span>
  );
}

export function BackupPanel() {
  const { jobs, status, error, loadJobs, loadStatus, pause, resume, clearError, loading } = useBackupStore();
  const [view, setView] = useState<View>({ kind: "jobs" });

  const refresh = useCallback(() => {
    void loadJobs();
    void loadStatus();
  }, [loadJobs, loadStatus]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Live progress: poll the queue while anything is queued/running, slowly otherwise.
  const active = status.tasks.length > 0;
  const wasActive = useRef(false);
  useEffect(() => {
    const t = setInterval(() => void loadStatus(), active ? 1000 : 10000);
    if (wasActive.current && !active) void loadJobs();
    wasActive.current = active;
    return () => clearInterval(t);
  }, [active, loadStatus, loadJobs]);

  // Keep a selected job in sync with refreshed data.
  const current = "job" in view ? jobs.find((j) => j.id === view.job.id) ?? view.job : null;

  const tabs: { kind: View["kind"]; label: string }[] = [
    { kind: "jobs", label: "Backups" },
    { kind: "history", label: "History" },
    { kind: "direct", label: "Restore from folder" },
    { kind: "recovery", label: "Recovery tool" },
    { kind: "settings", label: "Defaults & destinations" },
  ];

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-900" data-testid="backup-panel">
      <div className="flex items-center justify-between gap-2 p-3 border-b border-zinc-200 dark:border-zinc-700 flex-wrap">
        <h2 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Backups
          {current && <span className="font-normal text-zinc-500"> / {current.name || "New backup"}</span>}
        </h2>
        <div className="flex gap-1 flex-wrap">
          {tabs.map((t) => (
            <button
              key={t.kind}
              className={`px-2 py-1 text-xs rounded ${view.kind === t.kind ? "bg-blue-500 text-white" : "bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300"}`}
              onClick={() => setView({ kind: t.kind } as View)}
            >
              {t.label}
            </button>
          ))}
          <Btn onClick={() => setView({ kind: "import" })}>Import…</Btn>
          <Btn tone="success" onClick={() => setView({ kind: "edit", job: newBackupJob() })} testId="backup-new">
            + New backup
          </Btn>
        </div>
      </div>

      <div className="flex items-center justify-between gap-2 px-3 py-1 text-xs border-b border-zinc-200 dark:border-zinc-700">
        <span className={status.paused ? "text-amber-600" : "text-zinc-500"}>
          {status.paused
            ? `Scheduler paused${status.paused_until ? ` until ${formatDate(status.paused_until)}` : ""}`
            : status.next_runs[0]
              ? `Next: ${status.next_runs[0].job_name} at ${formatDate(status.next_runs[0].next_run_at)}`
              : "No scheduled backups"}
        </span>
        <DesktopToggles />
        {status.paused ? (
          <Btn onClick={() => void resume()}>Resume</Btn>
        ) : (
          <select
            className="text-xs rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800"
            value=""
            onChange={(e) => {
              const v = e.target.value;
              if (v) void pause(v === "forever" ? null : Number(v));
            }}
            aria-label="Pause scheduled backups"
          >
            <option value="">Pause…</option>
            <option value="30">30 minutes</option>
            <option value="60">1 hour</option>
            <option value="240">4 hours</option>
            <option value="1440">1 day</option>
            <option value="forever">Until resumed</option>
          </select>
        )}
      </div>

      <TaskQueue />
      {error && (
        <div className="p-2">
          <Banner tone="error" onClose={clearError}>{error}</Banner>
        </div>
      )}

      <div className="flex-1 overflow-y-auto">
        {view.kind === "jobs" && (
          <div className="p-2 space-y-2">
            {loading && jobs.length === 0 && <div className="text-sm text-zinc-500 text-center py-6">Loading…</div>}
            {!loading && jobs.length === 0 && (
              <div className="text-center text-sm text-zinc-500 py-8 space-y-2">
                <p>No backups yet.</p>
                <p className="text-xs">Versioned, deduplicated and encrypted backups to a folder, drive, NAS, SFTP, FTP, WebDAV or S3.</p>
                <Btn tone="success" onClick={() => setView({ kind: "edit", job: newBackupJob() })}>Create your first backup</Btn>
              </div>
            )}
            {jobs.length > 1 && <RunAllButton />}
            {jobs.map((j) => (
              <JobCard key={j.id} job={j} onView={setView} />
            ))}
          </div>
        )}
        {view.kind === "history" && <History />}
        {view.kind === "direct" && <DirectRestore />}
        {view.kind === "settings" && <BackupSettingsView />}
        {view.kind === "recovery" && <BackupRecoveryView />}
        {view.kind === "import" && <ImportConfig onDone={(j) => setView(j ? (j.id ? { kind: "jobs" } : { kind: "edit", job: j }) : { kind: "jobs" })} />}
        {view.kind === "edit" && current && (
          <BackupJobEditor
            key={current.id || "new"}
            initial={current}
            onDone={() => {
              refresh();
              setView({ kind: "jobs" });
            }}
          />
        )}
        {view.kind === "restore" && current && <BackupRestoreView job={current} />}
        {view.kind === "maintenance" && current && <BackupMaintenance job={current} onDeleted={() => setView({ kind: "jobs" })} />}
      </div>
    </div>
  );
}

export default BackupPanel;
