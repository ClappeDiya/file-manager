/**
 * Shared repositories (K4, K69), Maintenance › Sharing (Advanced): whether
 * the repository is shared, which computers and jobs write to it, and
 * "Share this repository with other computers/jobs" for a format 2 backup.
 */
import { useState } from "react";
import { formatDate } from "@/lib/backup-helpers";
import { errorText, useBackupStore, type BackupJob, type SharingReport, type SourceInfo } from "@/stores/backup-store";
import { Banner, Btn, Section } from "./backup-ui";

export function BackupSharing({ job }: { job: BackupJob }) {
  const execute = useBackupStore((s) => s.execute);
  const [state, setState] = useState<{ shared: boolean; sources: SourceInfo[] } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const refresh = () =>
    run(async () => {
      const r = await execute(job.id, { type: "sync_sources" });
      setState(r.report as unknown as { shared: boolean; sources: SourceInfo[] });
    });
  return (
    <Section
      title="Sharing"
      hint="A shared repository takes backups from several computers or jobs. Each keeps its own versions and retention; data they have in common is stored once."
    >
      <div data-testid="backup-sharing" className="space-y-2">
        {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
        {message && <Banner tone="success" onClose={() => setMessage(null)}>{message}</Banner>}
        <div className="flex gap-1 flex-wrap">
          <Btn disabled={busy} onClick={() => void refresh()} testId="sharing-refresh">
            {busy ? "Working…" : "Show computers and jobs"}
          </Btn>
          {state && !state.shared && (
            <Btn
              tone="primary"
              disabled={busy || job.read_only}
              testId="sharing-enable"
              onClick={() =>
                confirm("Share this repository? Other computers and jobs can then back up into it. Older app versions can no longer open it.") &&
                run(async () => {
                  const r = (await execute(job.id, { type: "enable_sharing", claim_all: true })).report as unknown as SharingReport;
                  setMessage(r.already_shared ? "The repository was already shared." : `The repository is shared; ${r.versions_assigned} existing version(s) now belong to this job.`);
                  const s = await execute(job.id, { type: "sync_sources" });
                  setState(s.report as unknown as { shared: boolean; sources: SourceInfo[] });
                })
              }
            >
              Share this repository with other computers/jobs
            </Btn>
          )}
        </div>
        {state && (
          <>
            <p className="text-xs">{state.shared ? "This repository is shared." : "Only this job writes to this repository."}</p>
            <ul className="text-[11px] border border-zinc-200 dark:border-zinc-700 rounded" data-testid="sharing-sources">
              {state.sources.map((s) => (
                <li key={s.id || "unknown"} className="flex justify-between gap-2 px-1">
                  <span className="truncate">
                    {s.own ? "● " : ""}
                    {s.label}
                  </span>
                  <span className="shrink-0 text-zinc-500">
                    {s.versions} version(s){s.newest ? `, newest ${formatDate(s.newest)}` : ""}
                  </span>
                </li>
              ))}
            </ul>
            {state.shared && <p className="text-[11px] text-zinc-500">To back up another computer into it, add a job there with the same destination and passphrase (or use Continue this backup on this computer).</p>}
          </>
        )}
      </div>
    </Section>
  );
}
