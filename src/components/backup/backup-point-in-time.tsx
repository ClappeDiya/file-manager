/**
 * "Open backup as it was at …" for jobs whose destination is a versioned
 * S3 bucket: creates a read-only job that reads every object as it was at
 * the chosen time (destination-level recovery, e.g. after ransomware
 * rewrote or deleted the backup's files). Restore from the new job as usual.
 */
import { useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { inputCls } from "@/lib/backup-helpers";
import { errorText, useBackupStore, type BackupJob } from "@/stores/backup-store";
import { Banner, Btn, Field, Section } from "./backup-ui";

export function PointInTimeOpen({ job }: { job: BackupJob }) {
  const [when, setWhen] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ tone: "success" | "error"; text: string } | null>(null);
  const dest = job.destination;
  if (dest.type !== "cloud" || dest.config.provider !== "s3" || job.read_only) return null;
  const open = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const at = new Date(when).toISOString();
      const created = await tauriInvoke<BackupJob>("backup_open_point_in_time", { jobId: job.id, at, name: name.trim() || null });
      await useBackupStore.getState().loadJobs();
      setMsg({ tone: "success", text: `Read-only job “${created.name}” created with ${created.stats.version_count} version(s) as of ${new Date(at).toLocaleString()}. Open it in the job list to restore.` });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section title="Open backup as it was at…" hint="Needs versioning on the bucket. Reads old object versions only; nothing is written to the bucket.">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2" data-testid="point-in-time">
        <Field label="Time">
          <input className={inputCls} type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} data-testid="pit-time" />
        </Field>
        <Field label="Name of the read-only job (optional)">
          <input className={inputCls} value={name} placeholder={`${job.name} (as of …)`} onChange={(e) => setName(e.target.value)} data-testid="pit-name" />
        </Field>
      </div>
      <Btn testId="pit-open" disabled={busy || !when || Number.isNaN(new Date(when).getTime())} onClick={() => void open()}>
        {busy ? "Reading old versions…" : "Open as read-only job"}
      </Btn>
      {msg && <Banner tone={msg.tone} onClose={() => setMsg(null)}>{msg.text}</Banner>}
    </Section>
  );
}
