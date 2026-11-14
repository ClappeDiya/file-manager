/**
 * Using a version without restoring it (Standard mode): "Download as ZIP…"
 * (K42), previewing one file in the app's preview pane (K38) and "Open as
 * drive" with the list of open drives (K44).
 */
import { useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { pickSaveFile } from "@/lib/folder-picker";
import { formatBytes } from "@/lib/format-bytes";
import { formatDate } from "@/lib/backup-helpers";
import { archiveFileName, executeRestore, exportBackupFile, previewBackupFile, restoreReport, useBackupDrives, type ExportedFile } from "@/stores/backup-access";
import { DEFAULT_RESTORE_OPTIONS, type BackupJob, type RestoreReport, type VersionSelector } from "@/stores/backup-store";
import { PreviewPane } from "@/components/preview-pane";
import { Banner, Btn } from "./backup-ui";

interface VersionRef {
  job: BackupJob;
  /** Version number (0 = newest) or a selector. */
  version: number | VersionSelector;
  timestamp: string;
  source: string | null;
}

/** "Download as ZIP…": the selection (or the whole version) into one archive. */
export function ArchiveExportButton({ job, version, timestamp, source, paths, onReport }: VersionRef & { paths: string[]; onReport: (r: RestoreReport) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <>
      <Btn
        disabled={busy}
        testId="backup-download-archive"
        title="Save the selected files (or the whole version) as a ZIP, TAR or TAR.GZ archive without restoring them"
        onClick={async () => {
          const path = await pickSaveFile("Save the backup as an archive", archiveFileName(job.name, timestamp), ["zip", "tar", "tgz", "gz"]);
          if (!path) return;
          setBusy(true);
          setError(null);
          try {
            const sel = typeof version === "number" ? { version, selector: null } : { version: 0, selector: version };
            const r = await executeRestore(job.id, { ...DEFAULT_RESTORE_OPTIONS, ...sel, paths, source, archive: { path, format: null } }, { download_bps: null, retries: null, network_timeout_secs: null });
            onReport(restoreReport(r));
          } catch (e) {
            setError(errorText(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "Saving…" : "Download as ZIP…"}
      </Btn>
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
    </>
  );
}

/** One backed-up file in the app's preview pane, with "Save a copy…". */
export function BackupFilePreview({ job, version, source, path, onClose }: Omit<VersionRef, "timestamp"> & { path: string; onClose: () => void }) {
  const [file, setFile] = useState<ExportedFile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setFile(null);
    setError(null);
    previewBackupFile(job.id, path, version, source)
      .then((f) => live && setFile(f))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [job.id, path, version, source]);
  const name = path.split(/[\\/]/).pop() ?? "file";
  return (
    <div className="fixed inset-0 z-50 bg-black/40 flex justify-end" role="dialog" aria-label={`Preview of ${name}`} data-testid="backup-file-preview">
      <div className="h-full flex flex-col bg-white dark:bg-zinc-900 shadow-xl">
        <div className="flex items-center justify-between gap-2 px-3 py-2 text-xs border-b border-zinc-200 dark:border-zinc-700">
          <span className="truncate" title={path}>
            {file ? `${name} · ${formatBytes(file.bytes)} · version from ${formatDate(file.timestamp)}` : `Loading ${name}…`}
          </span>
          <div className="flex gap-1 shrink-0">
            <Btn
              disabled={!file}
              onClick={async () => {
                const dest = await pickSaveFile("Save a copy of the backed-up file", name);
                if (!dest) return;
                try {
                  const r = await exportBackupFile(job.id, path, dest, version, source);
                  setSaved(`Saved ${formatBytes(r.bytes)} to ${r.local_path}`);
                } catch (e) {
                  setError(errorText(e));
                }
              }}
            >
              Save a copy…
            </Btn>
            <Btn onClick={onClose} testId="backup-file-preview-close">
              Close
            </Btn>
          </div>
        </div>
        {error && <Banner tone="error">{error}</Banner>}
        {saved && <Banner tone="success" onClose={() => setSaved(null)}>{saved}</Banner>}
        {file && <PreviewPane filePath={file.local_path} isOpen onToggle={onClose} width={480} />}
      </div>
    </div>
  );
}

/** "Open as drive": the version (or every version) as a read-only drive. */
export function OpenAsDriveButton({ job, version, source }: Omit<VersionRef, "timestamp">) {
  const open = useBackupDrives((s) => s.open);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const go = async (allVersions: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const sel = typeof version === "number" ? { version, selector: null } : { version: null, selector: version };
      await open(job.id, { ...sel, all_versions: allVersions, source });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Btn disabled={busy} testId="backup-open-drive" onClick={() => go(false)} title="Browse this version in your file manager as a read-only drive">
        {busy ? "Opening…" : "Open as drive"}
      </Btn>
      <Btn disabled={busy} onClick={() => go(true)} title="Every version as a folder named by its date and time">
        All versions as drive
      </Btn>
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
    </>
  );
}

/** Open backup drives with their address and an Unmount button. */
export function OpenDrivesList({ jobId }: { jobId?: string }) {
  const { drives, load, close } = useBackupDrives();
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    load().catch((e) => setError(errorText(e)));
  }, [load]);
  const shown = jobId ? drives.filter((d) => d.job_id === jobId) : drives;
  if (shown.length === 0 && !error) return null;
  return (
    <div className="space-y-1 text-xs" data-testid="backup-open-drives">
      <div className="font-medium">Open drives</div>
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      {shown.map((d) => (
        <div key={d.id} className="flex flex-wrap items-center gap-2 border border-zinc-200 dark:border-zinc-700 rounded p-1">
          <span className="flex-1 min-w-0">
            <span className="block truncate">
              {d.job_name}: {d.all_versions ? "all versions" : `version from ${formatDate(d.timestamp)}`}
              {d.mount_point ? ` — at ${d.mount_point}` : ""}
            </span>
            {!d.mount_point && (
              <span className="block text-zinc-500 break-all">
                Not mounted{d.mount_error ? ` (${d.mount_error})` : ""}. Connect to <span className="font-mono select-all">{d.url}</span> in your file manager (WebDAV; user {d.username}, password <span className="font-mono select-all">{d.password}</span>).
              </span>
            )}
          </span>
          <Btn onClick={() => navigator.clipboard?.writeText(d.url)} title="Copy the drive's address">
            Copy address
          </Btn>
          <Btn tone="danger" testId={`backup-unmount-${d.id}`} onClick={() => close(d.id).catch((e) => setError(errorText(e)))}>
            Unmount
          </Btn>
        </div>
      ))}
    </div>
  );
}
