/**
 * Legal hold on a backup job (retention suspended, nothing removable until
 * released; both recorded in the audit log) and the eDiscovery export: one
 * zip with the audit log, matching ledger events, the held backups' versions
 * and file lists and, optionally, the files themselves, hashed in a manifest.
 */
import { useState } from "react";
import {
  errorText,
  useBackupStore,
  type BackupJob,
  type EDiscoveryReport,
} from "@/stores/backup-store";
import { formatBytes } from "@/lib/format-bytes";
import { inputCls } from "@/lib/backup-helpers";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";

const lines = (v: string) =>
  v
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
/** `datetime-local` value → RFC 3339 (UTC), empty → null. */
const isoOrNull = (v: string) => (v ? new Date(v).toISOString() : null);

export function BackupLegalHold({ job }: { job: BackupJob }) {
  const { placeLegalHold, releaseLegalHold, exportEDiscovery } =
    useBackupStore();
  const hold = job.legal_hold;
  const [matter, setMatter] = useState("");
  const [reason, setReason] = useState("");
  const [output, setOutput] = useState("");
  const [paths, setPaths] = useState("");
  const [since, setSince] = useState("");
  const [until, setUntil] = useState("");
  const [includeFiles, setIncludeFiles] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<EDiscoveryReport | null>(null);

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section
      title="Legal hold"
      hint="While held, retention is suspended and no version, file or the job itself can be removed. Placing and releasing are recorded in the audit log."
    >
      {error && (
        <Banner tone="error" onClose={() => setError(null)}>
          {error}
        </Banner>
      )}
      {hold ? (
        <div className="space-y-2" data-testid="legal-hold-active">
          <Banner tone="warning">
            <div className="font-medium">Held for {hold.matter}</div>
            <div className="text-xs">
              {hold.reason && <>{hold.reason} · </>}placed{" "}
              {new Date(hold.placed_at).toLocaleString()} by {hold.placed_by}
            </div>
          </Banner>
          <Btn
            tone="danger"
            disabled={busy}
            testId="legal-hold-release"
            onClick={() =>
              confirm(
                `Release the legal hold on "${job.name}"? Retention applies again at the next backup.`,
              ) && void run(() => releaseLegalHold(job.id))
            }
          >
            Release hold
          </Btn>
        </div>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-[14rem_1fr_auto] gap-2 items-end">
          <Field label="Matter or case">
            <input
              className={inputCls}
              aria-label="Matter"
              value={matter}
              maxLength={200}
              onChange={(e) => setMatter(e.target.value)}
            />
          </Field>
          <Field label="Reason">
            <input
              className={inputCls}
              aria-label="Reason"
              value={reason}
              maxLength={2000}
              onChange={(e) => setReason(e.target.value)}
            />
          </Field>
          <Btn
            disabled={busy || !matter.trim()}
            testId="legal-hold-place"
            onClick={() =>
              void run(async () => {
                await placeLegalHold(job.id, matter, reason);
                setMatter("");
                setReason("");
              })
            }
          >
            Place hold
          </Btn>
        </div>
      )}

      <div
        className="pt-2 border-t border-zinc-200 dark:border-zinc-700 space-y-2"
        data-testid="ediscovery"
      >
        <div className="text-xs font-medium">eDiscovery export</div>
        <p className="text-xs text-zinc-500">
          A zip with the full audit log and its verification, file operations
          touching the paths, this job&apos;s versions and file lists and,
          optionally, the files of its newest version. Every part is hashed in
          manifest.json.
        </p>
        <Field label="Save as (.zip)">
          <PathInput
            value={output}
            onChange={setOutput}
            placeholder="/path/to/matter-export.zip"
            testId="ediscovery-output"
          />
        </Field>
        <Field label="Paths of interest (one per line, empty = everything)">
          <textarea
            className={`${inputCls} font-mono`}
            aria-label="Paths of interest"
            rows={2}
            value={paths}
            onChange={(e) => setPaths(e.target.value)}
          />
        </Field>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="File operations from">
            <input
              className={inputCls}
              aria-label="From"
              type="datetime-local"
              value={since}
              onChange={(e) => setSince(e.target.value)}
            />
          </Field>
          <Field label="Until">
            <input
              className={inputCls}
              aria-label="Until"
              type="datetime-local"
              value={until}
              onChange={(e) => setUntil(e.target.value)}
            />
          </Field>
        </div>
        <Check
          label="Include the files (restored from the newest version)"
          checked={includeFiles}
          onChange={setIncludeFiles}
        />
        <Btn
          disabled={busy || !output.trim() || !(hold?.matter ?? matter).trim()}
          title={
            hold
              ? undefined
              : "Give the matter above (the job does not need to be held)"
          }
          testId="ediscovery-export"
          onClick={() =>
            void run(async () => {
              setReport(null);
              setReport(
                await exportEDiscovery({
                  matter: hold?.matter ?? matter,
                  output: output.trim(),
                  paths: lines(paths),
                  since: isoOrNull(since),
                  until: isoOrNull(until),
                  job_ids: [job.id],
                  include_files: includeFiles,
                  max_file_bytes: null,
                }),
              );
            })
          }
        >
          {busy ? "Working…" : "Export"}
        </Btn>
        {report && (
          <Banner
            tone={
              report.audit_ok && !report.files_truncated ? "success" : "warning"
            }
            onClose={() => setReport(null)}
          >
            <div className="font-medium" data-testid="ediscovery-report">
              Saved {report.output} ({formatBytes(report.bytes)},{" "}
              {report.entries} entries, {report.ledger_events} file operations,{" "}
              {report.restored_files} files)
            </div>
            <div className="text-[11px] font-mono break-all">
              SHA-256 {report.sha256}
            </div>
            {!report.audit_ok && (
              <div className="text-xs">
                The audit log did not verify; see audit/verification.json.
              </div>
            )}
            {report.files_truncated && (
              <div className="text-xs">Files stopped at the size limit.</div>
            )}
          </Banner>
        )}
      </div>
    </Section>
  );
}
