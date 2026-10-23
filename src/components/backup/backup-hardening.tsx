/**
 * Ransomware protection of a backup job: a "Protection: 4/7" badge
 * (Standard), and in Advanced mode the checklist with what to change,
 * links to the hardening guide and "Run checks", which writes a probe
 * object to the destination and tries to delete it with the backup
 * credentials (only when asked).
 */
import { useEffect, useState } from "react";
import {
  errorText,
  useBackupStore,
  type BackupJob,
  type HardeningReport,
  type HardeningStatus,
} from "@/stores/backup-store";
import { DOCS_BASE, protectionTone } from "@/lib/hardening";
import { AdvancedOnly } from "./advanced-only";
import { Banner, Btn, Section } from "./backup-ui";

const ICON: Record<HardeningStatus, string> = {
  pass: "✓",
  warn: "!",
  fail: "✕",
  unknown: "?",
  info: "i",
};
const TONE: Record<HardeningStatus, string> = {
  pass: "text-green-700 dark:text-green-400",
  warn: "text-amber-700 dark:text-amber-400",
  fail: "text-red-700 dark:text-red-400",
  unknown: "text-zinc-500",
  info: "text-zinc-500",
};

export function BackupHardening({ job }: { job: BackupJob }) {
  const hardeningReport = useBackupStore((s) => s.hardeningReport);
  const [report, setReport] = useState<HardeningReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    if (!job.id) return;
    let live = true;
    hardeningReport(job.id)
      .then((r) => live && setReport(r))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [job.id, job.updated_at, hardeningReport]);

  if (!job.id) return null;
  const probe = async () => {
    setBusy(true);
    setError(null);
    setConfirming(false);
    try {
      setReport(await hardeningReport(job.id, true));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section
      title="Ransomware protection"
      hint="What keeps these backups safe if this computer and its credentials are taken over. Saved settings are checked."
    >
      {error && (
        <Banner tone="error" onClose={() => setError(null)}>
          {error}
        </Banner>
      )}
      {report && (
        <span
          data-testid="protection-badge"
          className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${protectionTone(report)}`}
          title={report.checks
            .filter((c) => c.status !== "pass" && c.status !== "info")
            .map((c) => c.title)
            .join("\n")}
        >
          Protection: {report.passed}/{report.total}
        </span>
      )}
      <AdvancedOnly hint>
        {report && (
          <ul
            className="mt-2 space-y-1.5 text-xs"
            data-testid="hardening-checks"
          >
            {report.checks.map((c) => (
              <li key={c.id} className="flex gap-2" data-status={c.status}>
                <span
                  aria-label={c.status}
                  className={`w-4 shrink-0 text-center font-bold ${TONE[c.status]}`}
                >
                  {ICON[c.status]}
                </span>
                <div>
                  <div className="font-medium">{c.title}</div>
                  <div className="text-zinc-600 dark:text-zinc-400">
                    {c.detail}
                  </div>
                  {c.advice && c.status !== "pass" && (
                    <div className="text-zinc-500">
                      {c.advice}{" "}
                      <a
                        className="underline"
                        href={DOCS_BASE + c.doc}
                        target="_blank"
                        rel="noreferrer"
                      >
                        Guide
                      </a>
                    </div>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
        {report?.probe?.leftover && (
          <Banner tone="warning">
            The probe object {report.probe.leftover} is still at the
            destination; remove it with the maintenance credentials.
          </Banner>
        )}
        <div className="mt-2 flex items-center gap-2">
          {confirming ? (
            <>
              <span className="text-xs">
                Writes a small probe file to the destination and tries to delete
                it with this job's credentials.
              </span>
              <Btn disabled={busy} onClick={() => void probe()}>
                Run now
              </Btn>
              <Btn onClick={() => setConfirming(false)}>Cancel</Btn>
            </>
          ) : (
            <Btn
              disabled={busy || job.read_only}
              title={
                job.read_only
                  ? "Read-only jobs never write to the destination"
                  : undefined
              }
              onClick={() => setConfirming(true)}
            >
              {busy ? "Checking…" : "Run checks"}
            </Btn>
          )}
          {report?.probe && (
            <span className="text-xs text-zinc-500">
              Last checked {new Date(report.probe.ran_at).toLocaleString()}
            </span>
          )}
        </div>
      </AdvancedOnly>
    </Section>
  );
}
