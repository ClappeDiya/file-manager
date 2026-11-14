import { useState } from "react";
import {
  ArrowRight,
  CheckCircle2,
  CircleAlert,
  FolderOpen,
  Pause,
  Play,
  RotateCcw,
  X,
} from "lucide-react";
import { Button } from "@ufop/ui-components";
import { tauriInvoke } from "@/hooks/use-tauri";
import {
  useTransferWorkspace,
  type WorkspaceJob,
} from "@/stores/transfer-workspace-store";
import { transferSummary } from "@/lib/transfer-progress";
import { formatBytes } from "@/lib/format-bytes";
import { errorText } from "@/lib/error-text";
import { useFileManagerStore, paneLabel } from "@/stores/file-manager-store";

export function TransferWorkspace({
  jobs: allJobs,
  error,
  onNewTransfer,
}: {
  jobs: WorkspaceJob[];
  error: string | null;
  onNewTransfer: () => void;
}) {
  const sessions = useTransferWorkspace((s) => s.sessions);
  const selectedId = useTransferWorkspace((s) => s.selectedId);
  const select = useTransferWorkspace((s) => s.select);
  const session = sessions.find((s) => s.id === selectedId);
  const jobs = session?.jobs ?? allJobs;
  error =
    error ??
    (jobs.some((j) => j.status_available === false)
      ? "This transfer is no longer in the current queue. Last known results are shown; check the destination files before starting a new transfer."
      : null);
  const result = transferSummary(jobs);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [detailsFor, setDetailsFor] = useState<string | null>(null);
  const reportScope = selectedId ?? "queue";
  const details = detailsFor === reportScope;
  const [report, setReport] = useState<{
    scope: string;
    message: string;
  } | null>(null);
  const hasWork = !!session || jobs.length > 0;
  const emptyFolders = !!session && !jobs.length;
  const done = result.settled || emptyFolders;
  const hasNotes = jobs.some(
    (j) => j.status === "completed" && j.error_message,
  );
  const status = !hasWork
    ? "Ready for a transfer"
    : error
      ? "Status unavailable"
      : result.failed
        ? done
          ? "Finished with failures"
          : "Transfer needs attention"
        : result.cancelled
          ? done
            ? "Transfer cancelled"
            : "Some files cancelled"
          : hasNotes
            ? done
              ? "Finished with warnings"
              : "Transfer has warnings"
            : done
              ? "Transfer completed"
              : result.percent === 100
                ? "Finalizing transfer"
                : jobs.some((j) => j.status === "conflict")
                  ? "Waiting for a conflict decision"
                  : jobs.some((j) => j.status === "paused") &&
                      !jobs.some((j) => j.status === "active")
                    ? "Transfer paused"
                    : "Transfer in progress";
  const action = async (command: string, jobIds: string[]) => {
    setBusy(true);
    setActionError(null);
    try {
      for (const jobId of jobIds) {
        const accepted = await tauriInvoke(command, { jobId });
        if (accepted === false)
          throw new Error(
            "The retry limit has been reached. Start a new transfer after correcting the problem.",
          );
      }
    } catch (e) {
      setActionError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const openFolders = () => {
    if (!session) return;
    const store = useFileManagerStore.getState();
    if (store.singlePaneMode) store.toggleSinglePaneMode();
    for (const [index, path] of [
      [0, session.sourceFolder || session.source],
      [1, session.destination],
    ] as const) {
      const tab = store.getActiveTab(index);
      if (tab) store.navigateTab(index, tab.id, path, paneLabel(path));
    }
    window.dispatchEvent(new CustomEvent("ufop:show-workspace"));
  };
  const saveReport = async () => {
    setActionError(null);
    try {
      const { pickSaveFile } = await import("@/lib/folder-picker");
      const path = await pickSaveFile(
        "Save transfer report",
        `transfer-${session?.id ?? "queue"}.json`,
        ["json"],
      );
      if (!path) return;
      await tauriInvoke("write_text_file_full", {
        path,
        content: JSON.stringify(
          {
            reportedAt: new Date().toISOString(),
            scope: session ? "selected transfer" : "current queue",
            statusFresh: !error,
            session: session ?? null,
            result,
            jobs,
          },
          null,
          2,
        ),
      });
      setReport({ scope: reportScope, message: `Report saved to ${path}` });
    } catch (e) {
      setActionError(errorText(e));
    }
  };
  return (
    <section
      aria-label="Transfer progress and results"
      className="shrink-0 border-t border-[var(--color-border)] bg-[var(--color-bg)] p-3 space-y-2 max-h-[40vh] overflow-auto"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          {result.failed || result.cancelled || hasNotes || error ? (
            <CircleAlert className="h-4 w-4 shrink-0" aria-hidden="true" />
          ) : done ? (
            <CheckCircle2 className="h-4 w-4 shrink-0" aria-hidden="true" />
          ) : (
            <ArrowRight className="h-4 w-4 shrink-0" aria-hidden="true" />
          )}
          <div role="status">
            <h2 className="text-sm font-semibold">{status}</h2>
          </div>
          {session && (
            <span className="text-xs text-[color:var(--color-text-secondary)] capitalize">
              {session.mode}
            </span>
          )}
        </div>
        <div className="flex items-center flex-wrap gap-2">
          {sessions.length > 0 && (
            <label className="text-xs">
              Transfer{" "}
              <select
                aria-label="Select transfer report"
                value={selectedId ?? ""}
                onChange={(e) => select(e.target.value || null)}
                className="max-w-48 rounded border border-[var(--color-border)] bg-[var(--color-bg)] p-1"
              >
                <option value="">Entire transfer queue</option>
                {sessions.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.source.split(/[/\\]/).pop()} ·{" "}
                    {new Date(s.createdAt).toLocaleTimeString()}
                  </option>
                ))}
              </select>
            </label>
          )}
          <Button size="sm" onClick={onNewTransfer}>
            New transfer
          </Button>
        </div>
      </div>
      {session?.kept && session.kept.length > 0 && (
        <p className="text-xs" role="status">
          {session.kept.length} converted document(s) were copied. Their
          originals remain at the source.
        </p>
      )}
      {session && (
        <div
          className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs"
          aria-label="Transfer locations"
        >
          <p className="break-all">
            <strong>Source</strong>{" "}
            {session.sources && session.sources.length > 1
              ? `${session.sources.length} selected items from ${session.sourceFolder}`
              : session.source}
          </p>
          <p className="break-all">
            <strong>Destination</strong> {session.destination}
          </p>
        </div>
      )}
      {hasWork && (
        <>
          <div className="flex justify-between flex-wrap gap-2 text-xs tabular-nums">
            <span>
              {result.completed} of {result.total} queued files completed
              {session && session.movedCount > 0
                ? ` · ${session.movedCount} items moved on the server`
                : ""}
              {emptyFolders
                ? ` · ${session.foldersCreated} folders created`
                : ""}
            </span>
            <span>
              {emptyFolders
                ? "Folders ready"
                : result.percent === null
                  ? "Calculating size…"
                  : `${result.percent}% transferred`}
              {result.speed > 0 && ` · ${formatBytes(result.speed)}/s`}
            </span>
          </div>
          <div
            role="progressbar"
            aria-label="Bytes transferred"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={emptyFolders ? 100 : (result.percent ?? undefined)}
            aria-valuetext={
              result.percent === null
                ? "Waiting for transfer size"
                : `${result.percent}% of bytes transferred; ${result.completed} of ${result.total} files completed`
            }
            className="h-2 overflow-hidden rounded-full bg-[var(--color-bg-tertiary)]"
          >
            <div
              className={`h-full rounded-full ${result.failed ? "bg-[var(--color-error)]" : "bg-[var(--color-primary)]"}`}
              style={{
                width: `${emptyFolders ? 100 : (result.percent ?? 0)}%`,
              }}
            />
          </div>
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
            <p>
              {formatBytes(result.bytesDone)} / {formatBytes(result.bytesTotal)}
              {done && result.total > 0 && (
                <>
                  {" "}
                  · <strong>{result.successPercent}% file success</strong>
                </>
              )}{" "}
              · {result.failed} failed · {result.cancelled} cancelled ·{" "}
              {result.verified} checksum verified
              {result.completed > result.verified &&
                ` · ${result.completed - result.verified} completed without checksum evidence`}
            </p>
            <div className="flex flex-wrap gap-1">
              {jobs.some(
                (j) =>
                  !j.external &&
                  j.status_available !== false &&
                  (j.status === "active" || j.status === "queued"),
              ) && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() =>
                    action(
                      "pause_transfer",
                      jobs
                        .filter(
                          (j) =>
                            !j.external &&
                            j.status_available !== false &&
                            ["active", "queued"].includes(j.status),
                        )
                        .map((j) => j.id),
                    )
                  }
                >
                  <Pause className="h-3 w-3 mr-1" />
                  Pause
                </Button>
              )}
              {jobs.some(
                (j) => j.status === "paused" && j.status_available !== false,
              ) && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() =>
                    action(
                      "resume_transfer",
                      jobs
                        .filter(
                          (j) =>
                            j.status === "paused" &&
                            j.status_available !== false,
                        )
                        .map((j) => j.id),
                    )
                  }
                >
                  <Play className="h-3 w-3 mr-1" />
                  Resume
                </Button>
              )}
              {!done &&
                jobs.some(
                  (j) =>
                    j.status_available !== false &&
                    !["completed", "failed", "cancelled"].includes(j.status),
                ) && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() =>
                      action(
                        "cancel_transfer",
                        jobs
                          .filter(
                            (j) =>
                              j.status_available !== false &&
                              !["completed", "failed", "cancelled"].includes(
                                j.status,
                              ),
                          )
                          .map((j) => j.id),
                      )
                    }
                  >
                    <X className="h-3 w-3 mr-1" />
                    Cancel
                  </Button>
                )}
              {jobs.some((j) => j.status === "failed" && !j.external) && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() =>
                    action(
                      "retry_transfer",
                      jobs
                        .filter((j) => j.status === "failed" && !j.external)
                        .map((j) => j.id),
                    )
                  }
                >
                  <RotateCcw className="h-3 w-3 mr-1" />
                  Retry failed
                </Button>
              )}
              {jobs.some(
                (j) => j.status === "conflict" && j.status_available !== false,
              ) && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    window.dispatchEvent(
                      new CustomEvent("ufop:show-workspace"),
                    );
                    setTimeout(
                      () =>
                        window.dispatchEvent(
                          new CustomEvent("ufop:open-transfer-queue"),
                        ),
                      0,
                    );
                  }}
                >
                  Resolve conflicts
                </Button>
              )}
              {session && (
                <Button size="sm" variant="ghost" onClick={openFolders}>
                  <FolderOpen className="h-3 w-3 mr-1" />
                  Show folders
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                aria-expanded={details}
                aria-controls="transfer-file-results"
                onClick={() => setDetailsFor(details ? null : reportScope)}
              >
                {details ? "Hide files" : "File results"}
              </Button>
              <Button size="sm" variant="ghost" onClick={saveReport}>
                Save report
              </Button>
            </div>
          </div>
          {details && (
            <ul
              id="transfer-file-results"
              aria-label="Individual file results"
              className="text-xs space-y-2"
            >
              {jobs.map((j) => (
                <li key={j.id} className="break-all">
                  <strong>{j.status}</strong> · {j.source_path} → {j.dest_path}
                  {j.error_message && <p>{j.error_message}</p>}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      {(error || actionError) && (
        <p role="alert" className="text-xs text-[color:var(--color-error)]">
          {actionError || error}
        </p>
      )}
      {report?.scope === reportScope && (
        <p role="status" className="text-xs break-all">
          {report.message}
        </p>
      )}
      {!hasWork && (
        <p className="text-xs text-[color:var(--color-text-secondary)]">
          Choose a source and destination, then start a transfer. Progress and
          the final report stay here.
        </p>
      )}
    </section>
  );
}
