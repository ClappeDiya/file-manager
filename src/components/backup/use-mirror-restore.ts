/**
 * Restore with the mirror safety flow (K45): a mirror restore first runs as
 * a dry run, shows the list of files it would delete, and runs for real only
 * after the user confirmed — through the Safety Interlock and with a
 * single-use confirmation token the backend checks.
 */
import { useState } from "react";
import { assessBeforeExecute } from "@/lib/safety";
import { deletionSummary, executeRestore, mirrorConfirmationToken, restoreReport } from "@/stores/backup-access";
import { useBackupStore, type RestoreOptions, type RestoreOverrides, type RestoreReport } from "@/stores/backup-store";

export interface PendingMirror {
  options: RestoreOptions;
  overrides: RestoreOverrides;
  deletions: string[];
}

export function useMirrorRestore(jobId: string, onReport: (r: RestoreReport) => void) {
  const [pending, setPending] = useState<PendingMirror | null>(null);

  const refresh = async () => {
    const s = useBackupStore.getState();
    await Promise.all([s.loadJobs(), s.loadStatus()]);
  };

  /** Run a restore; a mirror restore stops at the deletion list first. */
  const start = async (options: RestoreOptions, overrides: RestoreOverrides) => {
    if (!options.delete_extraneous || options.dry_run) {
      onReport(restoreReport(await executeRestore(jobId, options, overrides)));
      await refresh();
      return;
    }
    const dry = restoreReport(await executeRestore(jobId, { ...options, dry_run: true }, overrides));
    if (dry.planned_deletions.length === 0) {
      // Nothing to delete: an ordinary restore does the same.
      onReport(restoreReport(await executeRestore(jobId, { ...options, delete_extraneous: false }, overrides)));
      await refresh();
      return;
    }
    setPending({ options, overrides, deletions: dry.planned_deletions });
  };

  /** The user confirmed the deletion list. Returns false when the interlock was declined. */
  const confirm = async (): Promise<boolean> => {
    if (!pending) return false;
    const { options, overrides, deletions } = pending;
    const summary = `Mirror restore deletes ${deletionSummary(deletions)} in ${options.target ?? "the target"}`;
    const result = await assessBeforeExecute(
      { engine: "backup", kind: "mirror", affected_files: deletions.length, total_bytes: 0, subject_path: options.target, summary },
      async () => {
        const token = await mirrorConfirmationToken(summary);
        return executeRestore(jobId, options, { ...overrides, confirm_delete: true }, token);
      },
    );
    setPending(null);
    if (result === null) return false;
    onReport(restoreReport(result));
    await refresh();
    return true;
  };

  return { pending, start, confirm, cancel: () => setPending(null) };
}
