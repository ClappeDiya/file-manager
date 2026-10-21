/**
 * Per-job maintenance: verification, compaction, repair, database
 * recreation, broken-file inspection and purging, configuration export
 * (JSON / encrypted / command line) and controlled deletion.
 */
import { useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText, useBackupStore, type BackupJob, type BackupOperation, type OperationResult } from "@/stores/backup-store";
import { downloadText, inputCls } from "@/lib/backup-helpers";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";


export function BackupMaintenance({ job, onDeleted }: { job: BackupJob; onDeleted: () => void }) {
  const { execute, deleteJob } = useBackupStore();
  const [result, setResult] = useState<OperationResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [purgePaths, setPurgePaths] = useState("");
  const [includePass, setIncludePass] = useState(false);
  const [exportPass, setExportPass] = useState("");
  const [cli, setCli] = useState<string | null>(null);
  const [delLocal, setDelLocal] = useState(true);
  const [delRemote, setDelRemote] = useState(false);
  const [ctlVersion, setCtlVersion] = useState(0);
  const [ctlTarget, setCtlTarget] = useState("");

  const op = async (label: string, operation: BackupOperation) => {
    setBusy(label);
    setError(null);
    setResult(null);
    try {
      setResult(await execute(job.id, operation));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const tool = (label: string, operation: BackupOperation, hint: string, tone: "neutral" | "danger" = "neutral") => (
    <Btn tone={tone} disabled={busy !== null} title={hint} onClick={() => op(label, operation)} testId={`backup-op-${operation.type}`}>
      {busy === label ? "Working…" : label}
    </Btn>
  );

  return (
    <div className="p-3 space-y-3" data-testid="backup-maintenance">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      {result && (
        <Banner tone={result.status === "success" ? "success" : result.status === "warning" ? "warning" : "error"} onClose={() => setResult(null)}>
          <div className="font-medium">{result.summary}</div>
          <pre className="text-[11px] max-h-40 overflow-auto whitespace-pre-wrap">{JSON.stringify(result.report, null, 2)}</pre>
        </Banner>
      )}

      <Section title="Integrity">
        <div className="flex gap-1 flex-wrap">
          {tool("Verify sample", { type: "verify", full: false }, "Download a few volumes and check their hashes")}
          {tool("Verify everything", { type: "verify", full: true }, "Download every volume and check every block")}
          {tool("Consistency check", { type: "consistency" }, "Compare the local index with the destination listing")}
          {tool("List broken files", { type: "list_broken" }, "Files that depend on missing or damaged volumes")}
        </div>
      </Section>

      <Section title="Repair">
        <div className="flex gap-1 flex-wrap">
          {tool("Repair", { type: "repair" }, "Re-upload missing index/file-list volumes from the local database")}
          {tool("Recreate database", { type: "recreate_database" }, "Rebuild the local database from the destination")}
          {tool("Compact now", { type: "compact" }, "Delete unused data and repack sparse volumes")}
          {tool("Preview purge of broken files", { type: "purge_broken", dry_run: true }, "Show what would be removed")}
          {tool("Rebuild missing volumes", { type: "rebuild_missing" }, "Re-create lost data volumes from the source files still on this computer")}
          {tool("Purge broken files", { type: "purge_broken", dry_run: false }, "Remove unrecoverable files from history so backups can continue", "danger")}
        </div>
      </Section>

      <Section title="Snapshot diagnostics" hint="Creates and immediately removes a test snapshot of this job's sources, listing every step.">
        {tool("Test snapshot", { type: "snapshot_test" }, "Check that VSS / LVM / APFS snapshots work for these sources")}
      </Section>

      <Section title="Restore tuning" hint="Downloads a sample of volumes with 1, 2, 4 and 8 parallel readers (nothing is written) and reports the best setting for restores.">
        {tool("Run restore AutoTune", { type: "auto_tune", sample_volumes: 8 }, "Benchmark restore download concurrency")}
      </Section>

      <Section title="Control files" hint="Write the control files stored with a version into a folder.">
        <div className="grid grid-cols-1 sm:grid-cols-[8rem_1fr] gap-2">
          <Field label="Version (0 = newest)">
            <input className={inputCls} type="number" min={0} value={ctlVersion} onChange={(e) => setCtlVersion(Math.max(0, Math.floor(Number(e.target.value) || 0)))} />
          </Field>
          <Field label="Target folder">
            <PathInput value={ctlTarget} onChange={setCtlTarget} placeholder="/path/to/folder" testId="control-target" />
          </Field>
        </div>
        <Btn disabled={!ctlTarget.trim() || busy !== null} testId="backup-op-extract_control_files" onClick={() => op("Extract control files", { type: "extract_control_files", version: ctlVersion, target: ctlTarget.trim() })}>
          {busy === "Extract control files" ? "Working…" : "Extract control files"}
        </Btn>
      </Section>

      <Section title="Remove files from history" hint="Paths, folders or wildcards (one per line). Removes them from every version.">
        <textarea className={`${inputCls} font-mono`} rows={2} value={purgePaths} onChange={(e) => setPurgePaths(e.target.value)} />
        <div className="flex gap-1">
          {(() => {
            const paths = purgePaths.split("\n").map((s) => s.trim()).filter(Boolean);
            return (
              <>
                <Btn disabled={!paths.length || busy !== null} onClick={() => op("Preview purge", { type: "purge", paths, versions: null, dry_run: true })}>Preview</Btn>
                <Btn tone="danger" disabled={!paths.length || busy !== null} onClick={() => confirm("Permanently remove these paths from all versions?") && op("Purge", { type: "purge", paths, versions: null, dry_run: false })}>
                  Purge
                </Btn>
              </>
            );
          })()}
        </div>
      </Section>

      <Section title="Export configuration">
        <Check label="Include the backup passphrase" checked={includePass} onChange={setIncludePass} hint="Only with an export passphrase, unless you store the file securely." />
        <Field label="Encrypt the export with (optional)">
          <input className={inputCls} type="password" value={exportPass} onChange={(e) => setExportPass(e.target.value)} />
        </Field>
        <div className="flex gap-1 flex-wrap">
          <Btn
            onClick={async () => {
              if (includePass && !exportPass && !confirm("Export the passphrase in plain text?")) return;
              try {
                const json = await tauriInvoke<string>("backup_export_config", { id: job.id, includePassphrase: includePass, exportPassphrase: exportPass || null, includeStats: true });
                downloadText(`${job.name.replace(/[^\w.-]+/g, "_")}-backup.json`, json);
              } catch (e) {
                setError(errorText(e));
              }
            }}
          >
            Download JSON
          </Btn>
          <Btn
            onClick={async () => {
              try {
                setCli(await tauriInvoke<string>("backup_export_cli", { id: job.id }));
              } catch (e) {
                setError(errorText(e));
              }
            }}
          >
            Show command line
          </Btn>
        </div>
        {cli && (
          <div className="space-y-1">
            <pre className="text-[11px] whitespace-pre-wrap break-all bg-zinc-50 dark:bg-zinc-800 p-2 rounded" data-testid="backup-cli">{cli}</pre>
            <Btn onClick={() => navigator.clipboard?.writeText(cli)}>Copy</Btn>
          </div>
        )}
      </Section>

      <Section title="Delete job">
        <Check label="Also delete the local database" checked={delLocal} onChange={setDelLocal} />
        <Check label="Also delete all backup data at the destination" checked={delRemote} onChange={setDelRemote} hint="Irreversible: every version of this backup is lost." />
        <Btn
          tone="danger"
          disabled={busy !== null}
          testId="backup-delete-job"
          onClick={async () => {
            if (!confirm(delRemote ? `Delete "${job.name}" AND all of its backup data?` : `Delete the job "${job.name}"? Backup data stays at the destination.`)) return;
            try {
              await deleteJob(job.id, delLocal, delRemote);
              onDeleted();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Delete job
        </Btn>
      </Section>
    </div>
  );
}
