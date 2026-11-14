/**
 * Extra destinations ("replicas"): full copies of the backup mirrored after
 * each backup, on their own interval, or on demand. Each replica can be
 * restored from on its own (Restore from folder).
 */
import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { formatLocalTime } from "@/lib/history-range";
import { runDuration } from "@/lib/backup-time";
import { triggerLabel } from "@/lib/run-trigger";
import { inputCls } from "@/lib/backup-helpers";
import type { BackupDestination, Replica, ReplicaRun, ReplicaSchedule, SavedDestination } from "@/stores/backup-store";
import { Btn, Check, Field, PathInput, Section } from "./backup-ui";
import { CloudDestinationForm, newCloudConfig } from "./backup-cloud-destination";

interface Conn {
  id: string;
  name: string;
  protocol: string;
}

function newReplica(n: number): Replica {
  return {
    id: "",
    name: `Copy ${n}`,
    destination: { type: "local", path: "", alternate_paths: [], marker_file: null },
    enabled: true,
    schedule: { type: "after_backup" },
    delete_extraneous: true,
    verify: false,
    throttle: { upload_bps: 0, download_bps: 0 },
    last_run: null,
    last_result: null,
  };
}

function DestinationPicker({ value, onChange, connections, library }: { value: BackupDestination; onChange: (d: BackupDestination) => void; connections: Conn[]; library: SavedDestination[] }) {
  return (
    <div className="space-y-1">
      <select
        className={inputCls}
        value={value.type}
        onChange={(e) =>
          onChange(
            e.target.value === "local"
              ? { type: "local", path: "", alternate_paths: [], marker_file: null }
              : e.target.value === "connection"
                ? { type: "connection", connection_id: "", remote_path: "" }
                : e.target.value === "cloud"
                  ? { type: "cloud", config: newCloudConfig("s3"), credential_id: crypto.randomUUID() }
                  : { type: "library", destination_id: "", remote_path: "" },
          )
        }
      >
        <option value="local">Folder / drive / mounted share</option>
        <option value="connection">Saved connection</option>
        <option value="library">Destination library</option>
        <option value="cloud">Cloud / network service</option>
      </select>
      {value.type === "cloud" && <CloudDestinationForm config={value.config} credentialId={value.credential_id} onChange={(c) => onChange({ ...value, config: c })} />}
      {value.type === "local" && <PathInput value={value.path} onChange={(p) => onChange({ ...value, path: p })} placeholder="/Volumes/Offsite" />}
      {value.type === "connection" && (
        <div className="grid grid-cols-2 gap-1">
          <select className={inputCls} value={value.connection_id} onChange={(e) => onChange({ ...value, connection_id: e.target.value })}>
            <option value="">Choose…</option>
            {connections.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} ({c.protocol})
              </option>
            ))}
          </select>
          <input className={inputCls} placeholder="remote folder" value={value.remote_path} onChange={(e) => onChange({ ...value, remote_path: e.target.value })} />
        </div>
      )}
      {value.type === "library" && (
        <div className="grid grid-cols-2 gap-1">
          <select className={inputCls} value={value.destination_id} onChange={(e) => onChange({ ...value, destination_id: e.target.value })}>
            <option value="">Choose…</option>
            {library.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
          <input className={inputCls} placeholder="sub-folder" value={value.remote_path} onChange={(e) => onChange({ ...value, remote_path: e.target.value })} />
        </div>
      )}
    </div>
  );
}

/** The last copies made to one replica: every run, not only the latest result. */
export function ReplicaHistory({ jobId, replicaId }: { jobId: string; replicaId: string }) {
  const [rows, setRows] = useState<ReplicaRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    tauriInvoke<ReplicaRun[]>("backup_replica_history", { jobId, replicaId, limit: 30 })
      .then((r) => live && setRows(r))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [jobId, replicaId]);
  if (error) return <div className="text-xs text-red-600" role="alert">{error}</div>;
  if (!rows) return <div className="text-xs text-zinc-500">Loading…</div>;
  if (rows.length === 0) return <div className="text-xs text-zinc-500">No copies to this destination yet.</div>;
  return (
    <table className="w-full text-[11px]" data-testid="replica-history">
      <thead>
        <tr className="text-left text-zinc-500">
          <th>When</th>
          <th>Result</th>
          <th>Files</th>
          <th>Size</th>
          <th>Took</th>
          <th>Started by</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id}>
            <td>{formatLocalTime(r.started_at)}</td>
            <td className={r.status === "success" ? "text-green-600" : "text-red-600"} title={r.error ?? undefined}>
              {r.status}
              {r.error ? ` — ${r.error}` : ""}
            </td>
            <td>{r.files}</td>
            <td>{formatBytes(r.bytes)}</td>
            <td>{runDuration(r.started_at, r.finished_at)}</td>
            <td>{triggerLabel(r.trigger)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function BackupReplicas({
  replicas,
  onChange,
  connections,
  library,
  jobId,
}: {
  replicas: Replica[];
  onChange: (r: Replica[]) => void;
  connections: Conn[];
  library: SavedDestination[];
  /** The saved job these replicas belong to; without it (a new job) there is no history to show. */
  jobId?: string;
}) {
  const [historyOf, setHistoryOf] = useState<Set<string>>(new Set());
  const toggleHistory = (id: string) =>
    setHistoryOf((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const set = (i: number, p: Partial<Replica>) => onChange(replicas.map((r, j) => (j === i ? { ...r, ...p } : r)));
  return (
    <Section title="Extra destinations (replicas)" hint="Full copies of this backup on other destinations, kept in sync. Each one can be restored from on its own.">
      {replicas.map((r, i) => (
        <div key={r.id || i} className="border border-zinc-200 dark:border-zinc-700 rounded p-2 space-y-1" data-testid="replica">
          <div className="flex gap-2 items-end">
            <Field label="Name">
              <input className={inputCls} value={r.name} onChange={(e) => set(i, { name: e.target.value })} />
            </Field>
            <Field label="When">
              <select
                className={inputCls}
                value={r.schedule.type}
                onChange={(e) => {
                  const t = e.target.value as ReplicaSchedule["type"];
                  set(i, { schedule: t === "interval" ? { type: "interval", minutes: 60 } : ({ type: t } as ReplicaSchedule) });
                }}
              >
                <option value="after_backup">After every backup</option>
                <option value="interval">On an interval</option>
                <option value="manual">Only manually</option>
              </select>
            </Field>
            {r.schedule.type === "interval" && (
              <Field label="Every (minutes)">
                <input
                  className={inputCls}
                  type="number"
                  min={5}
                  value={r.schedule.minutes}
                  onChange={(e) => set(i, { schedule: { type: "interval", minutes: Math.max(5, Number(e.target.value) || 5) } })}
                />
              </Field>
            )}
            <Btn tone="danger" onClick={() => onChange(replicas.filter((_, j) => j !== i))}>
              Remove
            </Btn>
          </div>
          <DestinationPicker value={r.destination} onChange={(d) => set(i, { destination: d })} connections={connections} library={library} />
          <div className="flex gap-3 flex-wrap">
            <Check label="Enabled" checked={r.enabled} onChange={(v) => set(i, { enabled: v })} />
            <Check label="Remove deleted versions from the copy" checked={r.delete_extraneous} onChange={(v) => set(i, { delete_extraneous: v })} />
            <Check label="Read back and verify every copied file" checked={r.verify} onChange={(v) => set(i, { verify: v })} />
          </div>
          <Field label="Upload limit (KB/s, 0 = unlimited)">
            <input className={inputCls} type="number" min={0} value={Math.round(r.throttle.upload_bps / 1024)} onChange={(e) => set(i, { throttle: { ...r.throttle, upload_bps: Math.max(0, Number(e.target.value)) * 1024 } })} />
          </Field>
          {r.last_run && (
            <div className="text-xs text-zinc-500">
              Last copy {formatLocalTime(r.last_run)}: {r.last_result}
            </div>
          )}
          {jobId && r.id && (
            <div className="space-y-1">
              <Btn testId={`replica-history-toggle-${r.id}`} onClick={() => toggleHistory(r.id)}>
                {historyOf.has(r.id) ? "Hide history" : "Show history"}
              </Btn>
              {historyOf.has(r.id) && <ReplicaHistory jobId={jobId} replicaId={r.id} />}
            </div>
          )}
        </div>
      ))}
      <Btn testId="replica-add" onClick={() => onChange([...replicas, newReplica(replicas.length + 1)])}>
        + Add extra destination
      </Btn>
    </Section>
  );
}
