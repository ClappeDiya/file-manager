/**
 * Retention step of the job editor (K20, K23): the policy choice, the
 * "Keep daily / weekly / monthly" GFS preset (Standard) with its count
 * editor (Advanced), and a live preview of which existing versions the
 * policy, as edited and not yet saved, keeps and why.
 */
import { useEffect, useState } from "react";
import { formatDate, inputCls } from "@/lib/backup-helpers";
import { describeCounts, errorText, GFS_PRESET, useBackupStore, type BackupJob, type RetentionPolicy, type RetentionPreview } from "@/stores/backup-store";
import { AdvancedOnly } from "./advanced-only";
import { Banner, Field, Section } from "./backup-ui";

type Counts = Extract<RetentionPolicy, { type: "counts" }>;

const COUNT_FIELDS: [keyof Omit<Counts, "type">, string][] = [
  ["keep_latest", "Latest versions"],
  ["keep_hourly", "Hourly"],
  ["keep_daily", "Daily"],
  ["keep_weekly", "Weekly (ISO weeks)"],
  ["keep_monthly", "Monthly"],
  ["keep_yearly", "Yearly"],
];

function defaultFor(t: RetentionPolicy["type"]): RetentionPolicy {
  switch (t) {
    case "keep_days":
      return { type: t, days: 30 };
    case "keep_count":
      return { type: t, count: 10 };
    case "custom":
      return { type: t, rules: "1W:1D,4W:1W,12M:1M" };
    case "counts":
      return GFS_PRESET;
    default:
      return { type: t };
  }
}

/** Which existing versions the (unsaved) policy keeps. */
export function RetentionPreviewList({ jobId, policy }: { jobId: string; policy: RetentionPolicy }) {
  const previewRetention = useBackupStore((s) => s.previewRetention);
  const [preview, setPreview] = useState<RetentionPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const key = JSON.stringify(policy);
  useEffect(() => {
    let live = true;
    const t = setTimeout(() => {
      previewRetention(jobId, JSON.parse(key) as RetentionPolicy)
        .then((p) => live && (setPreview(p), setError(null)))
        .catch((e) => live && setError(errorText(e)));
    }, 300);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [jobId, key, previewRetention]);
  if (error) return <Banner tone="warning">{error}</Banner>;
  if (!preview) return <p className="text-xs text-zinc-500">Checking the existing versions…</p>;
  if (preview.versions.length === 0) return <p className="text-xs text-zinc-500">No versions yet.</p>;
  return (
    <div className="space-y-1" data-testid="retention-preview">
      <p className="text-xs">
        With this setting <b>{preview.keep}</b> version(s) are kept and <b className={preview.delete ? "text-red-600" : ""}>{preview.delete}</b> deleted after the next backup.
        {preview.legal_hold && " The job is under legal hold: nothing is deleted."}
      </p>
      <ul className="max-h-48 overflow-y-auto text-[11px] border border-zinc-200 dark:border-zinc-700 rounded">
        {preview.versions.map((v) => (
          <li key={v.id} className={`flex justify-between gap-2 px-1 ${v.keep ? "" : "text-red-600 line-through decoration-1"}`}>
            <span className="whitespace-nowrap">{formatDate(v.timestamp)}</span>
            <span className="truncate text-right" title={v.summary}>
              {v.summary}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function RetentionEditor({ job, isNew, onChange }: { job: BackupJob; isNew: boolean; onChange: (r: RetentionPolicy) => void }) {
  const r = job.retention;
  return (
    <Section title="Version retention" hint="Applied after each successful backup. The newest version and versions set to Keep forever are never deleted.">
      <select className={inputCls} value={r.type} onChange={(e) => onChange(defaultFor(e.target.value as RetentionPolicy["type"]))} data-testid="backup-retention">
        <option value="keep_all">Keep all versions</option>
        <option value="counts">Keep daily, weekly, monthly and yearly versions</option>
        <option value="keep_days">Delete versions older than…</option>
        <option value="keep_count">Keep a number of versions</option>
        <option value="smart">Smart (one per day for a week, per week for a month, per month for a year)</option>
        <option value="custom">Custom thinning rules</option>
      </select>
      {r.type === "keep_days" && (
        <Field label="Days">
          <input className={inputCls} type="number" min={1} value={r.days} onChange={(e) => onChange({ type: "keep_days", days: Math.max(1, Number(e.target.value)) })} />
        </Field>
      )}
      {r.type === "keep_count" && (
        <Field label="Versions">
          <input className={inputCls} type="number" min={1} value={r.count} onChange={(e) => onChange({ type: "keep_count", count: Math.max(1, Number(e.target.value)) })} />
        </Field>
      )}
      {r.type === "custom" && (
        <Field label="Rules (timeframe:interval, …)" hint="Units s, m, h, D, W, M, Y; interval U keeps all. Example: 7D:U,4W:1D,12M:1W">
          <input className={`${inputCls} font-mono`} value={r.rules} onChange={(e) => onChange({ type: "custom", rules: e.target.value })} />
        </Field>
      )}
      {r.type === "counts" && (
        <>
          <p className="text-xs text-zinc-600 dark:text-zinc-300" data-testid="retention-counts-summary">
            {describeCounts(r)}: for each period the newest version of each of the most recent periods is kept (UTC; weeks start on Monday).
          </p>
          <AdvancedOnly hint>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
              {COUNT_FIELDS.map(([k, label]) => (
                <Field key={k} label={label}>
                  <input
                    className={inputCls}
                    type="number"
                    min={0}
                    max={100000}
                    aria-label={`Keep ${label.toLowerCase()}`}
                    value={r[k]}
                    onChange={(e) => onChange({ ...r, [k]: Math.min(100000, Math.max(0, Math.floor(Number(e.target.value) || 0))) })}
                  />
                </Field>
              ))}
            </div>
          </AdvancedOnly>
        </>
      )}
      {!isNew && job.id && <RetentionPreviewList jobId={job.id} policy={r} />}
    </Section>
  );
}
