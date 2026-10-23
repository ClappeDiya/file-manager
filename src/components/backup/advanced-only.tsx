/**
 * Simple/Advanced gate for the Backup screens (K132).
 *
 * `useIsAdvanced()` is true when the per-area override `backupAdvanced` in the
 * UI store is true, or when it is null and the global app mode is "advanced".
 * `<AdvancedOnly>` renders its children only then; in Simple mode it renders
 * nothing, or a one-line hint with a button that switches to Advanced mode.
 *
 * What stays visible in Simple mode (the everyday flow):
 * - Job list: run, dry run, restore, edit, maintenance, history, import.
 * - Job editor: General (name, passphrase), Sources, Filters, Destination,
 *   Schedule, Retention, Options › Storage + Network limits, Hooks › desktop
 *   and channel notifications.
 * - Restore: versions, search, file picker, restore, restore from location.
 * - Maintenance: Integrity checks, export configuration, delete job.
 * - Settings: destination library, basic defaults, machine-wide limits, and
 *   the "Show advanced backup options" toggle itself.
 * - Recovery: the download / index / restore / convert steps.
 *
 * Advanced only (shown exactly as before in Advanced mode):
 * - Job editor: file-name prefix, extra destinations (replicas), advanced
 *   options, source handling, integrity and maintenance options, scripts,
 *   HTTP reports, log outputs, secret providers.
 * - Maintenance: legal hold, repair/recreate/compact/purge-broken, replica
 *   updates, object locks, destination file browser (raw remote listing),
 *   snapshot diagnostics, support tools, restore tuning, control files,
 *   remove files from history.
 * - Settings: local database tools, secret providers, advanced defaults.
 * - Recovery: file-crypt / GPG module tools.
 */
import type { ReactNode } from "react";
import { selectBackupAdvanced, useUIStore } from "@/stores/ui-store";

/** True when the Backup screens should show their advanced sections. */
export function useIsAdvanced(): boolean {
  return useUIStore(selectBackupAdvanced);
}

export function AdvancedOnly({ children, hint = false }: { children: ReactNode; hint?: boolean }) {
  const advanced = useIsAdvanced();
  const setAppMode = useUIStore((s) => s.setAppMode);
  const setBackupAdvanced = useUIStore((s) => s.setBackupAdvanced);
  if (advanced) return <>{children}</>;
  if (!hint) return null;
  return (
    <p className="text-xs text-zinc-500 dark:text-zinc-400 flex items-center gap-2" data-testid="advanced-hint">
      More options are available in Advanced mode.
      <button
        type="button"
        className="text-blue-600 dark:text-blue-400 hover:underline"
        onClick={() => {
          // An explicit "simple" override would keep hiding things; clear it.
          setBackupAdvanced(null);
          setAppMode("advanced");
        }}
      >
        Switch to Advanced
      </button>
    </p>
  );
}
