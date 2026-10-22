/**
 * Backup result reports from machines, the per-backup rollup, and the
 * aggregates behind the fleet dashboards.
 *
 * `normalizeReport` accepts both the agent's report and the body the client
 * sends to plain report URLs (`{name, operation, result, backup_id, duration,
 * data: BackupReport}`), so clients can report without the agent.
 */
import { db, newId, now, parseJson, type Row } from '../index';
import { enqueueEvent } from './webhooks';

export interface NormalizedReport {
  backupId: string;
  backupName: string;
  operation: string;
  status: 'success' | 'warning' | 'error' | 'cancelled';
  startedAt: string | null;
  finishedAt: string;
  durationMs: number;
  filesExamined: number;
  filesAdded: number;
  filesModified: number;
  filesDeleted: number;
  bytesUploaded: number;
  sourceSize: number;
  backendSize: number;
  versionCount: number;
  warnings: number;
  errors: number;
  message: string;
  scheduleIntervalMinutes: number | null;
}

const num = (v: unknown) => {
  const n = typeof v === 'number' ? v : typeof v === 'string' ? parseFloat(v) : NaN;
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : 0;
};
const str = (v: unknown, n = 200) => (typeof v === 'string' ? v.slice(0, n) : '');
const count = (v: unknown) => (Array.isArray(v) ? v.length : num(v));
const iso = (v: unknown): string | null => {
  if (typeof v !== 'string') return null;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d.toISOString();
};

/** Validate and map a raw payload; throws on reports that cannot be attributed. */
export function normalizeReport(raw: Record<string, unknown>): NormalizedReport {
  const data = (raw.data && typeof raw.data === 'object' ? raw.data : raw.report && typeof raw.report === 'object' ? raw.report : raw) as Record<string, unknown>;
  const stats = (raw.stats && typeof raw.stats === 'object' ? raw.stats : {}) as Record<string, unknown>;
  const backupId = str(raw.backup_id ?? raw.backupId ?? raw.job_id, 120);
  if (!backupId) throw new Error('The report has no backup id');
  const statusRaw = str(raw.result ?? raw.status, 20).toLowerCase();
  const status = (['success', 'warning', 'error', 'cancelled'].includes(statusRaw) ? statusRaw : statusRaw === 'failed' ? 'error' : 'success') as NormalizedReport['status'];
  const duration = num(data.duration_ms ?? raw.duration_ms) || num(String(raw.duration ?? '').replace(/s$/, '')) * 1000;
  const finishedAt = iso(raw.finished_at ?? raw.finishedAt) ?? now();
  return {
    backupId,
    backupName: str(raw.name ?? raw.backup_name, 200) || backupId,
    operation: str(raw.operation, 40) || 'backup',
    status,
    startedAt: iso(raw.started_at ?? raw.startedAt),
    finishedAt,
    durationMs: duration,
    filesExamined: num(data.files_examined),
    filesAdded: num(data.files_added),
    filesModified: num(data.files_changed ?? data.files_modified),
    filesDeleted: num(data.files_deleted),
    bytesUploaded: num(data.bytes_uploaded),
    sourceSize: num(stats.source_size ?? data.bytes_examined),
    backendSize: num(stats.backend_size),
    versionCount: num(stats.version_count),
    warnings: count(data.warnings),
    errors: count(data.errors),
    message: str(raw.message, 2000),
    scheduleIntervalMinutes: num(stats.schedule_interval_minutes) || null,
  };
}

export interface StoredReport extends NormalizedReport {
  id: string;
  orgId: string;
  machineId: string;
  receivedAt: string;
}

function fromRow(r: Row): StoredReport {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    machineId: r.machine_id as string,
    backupId: r.backup_id as string,
    backupName: r.backup_name as string,
    operation: r.operation as string,
    status: r.status as StoredReport['status'],
    startedAt: (r.started_at as string) ?? null,
    finishedAt: r.finished_at as string,
    durationMs: Number(r.duration_ms),
    filesExamined: Number(r.files_examined),
    filesAdded: Number(r.files_added),
    filesModified: Number(r.files_modified),
    filesDeleted: Number(r.files_deleted),
    bytesUploaded: Number(r.bytes_uploaded),
    sourceSize: Number(r.source_size),
    backendSize: Number(r.backend_size),
    versionCount: Number(r.version_count),
    warnings: Number(r.warnings),
    errors: Number(r.errors),
    message: r.message as string,
    scheduleIntervalMinutes: null,
    receivedAt: r.received_at as string,
  };
}

export function ingestReport(orgId: string, machineId: string, rep: NormalizedReport, raw: unknown): StoredReport {
  const id = newId('rpt');
  const d = db();
  d.transaction(() => {
    d.prepare(`INSERT INTO backup_reports (id, org_id, machine_id, backup_id, backup_name, operation, status, started_at, finished_at, duration_ms, files_examined, files_added, files_modified, files_deleted, bytes_uploaded, source_size, backend_size, version_count, warnings, errors, message, received_at, raw)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`).run(
      id, orgId, machineId, rep.backupId, rep.backupName, rep.operation, rep.status, rep.startedAt, rep.finishedAt, rep.durationMs, rep.filesExamined, rep.filesAdded, rep.filesModified, rep.filesDeleted, rep.bytesUploaded, rep.sourceSize, rep.backendSize, rep.versionCount, rep.warnings, rep.errors, rep.message, now(), JSON.stringify(raw).slice(0, 200_000),
    );
    if (rep.operation === 'backup') {
      const ok = rep.status === 'success' || rep.status === 'warning';
      d.prepare(`INSERT INTO backups (org_id, machine_id, backup_id, name, last_status, last_run_at, last_success_at, schedule_interval_minutes, source_size, backend_size, version_count)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(machine_id, backup_id) DO UPDATE SET org_id = excluded.org_id, name = excluded.name, last_status = excluded.last_status, last_run_at = excluded.last_run_at,
                   last_success_at = COALESCE(excluded.last_success_at, backups.last_success_at),
                   schedule_interval_minutes = COALESCE(excluded.schedule_interval_minutes, backups.schedule_interval_minutes),
                   source_size = CASE WHEN excluded.source_size > 0 THEN excluded.source_size ELSE backups.source_size END,
                   backend_size = CASE WHEN excluded.backend_size > 0 THEN excluded.backend_size ELSE backups.backend_size END,
                   version_count = CASE WHEN excluded.version_count > 0 THEN excluded.version_count ELSE backups.version_count END`).run(
        orgId, machineId, rep.backupId, rep.backupName, rep.status, rep.finishedAt, ok ? rep.finishedAt : null, rep.scheduleIntervalMinutes, rep.sourceSize, rep.backendSize, rep.versionCount,
      );
    }
    d.prepare('UPDATE machines SET last_report_at = ? WHERE id = ?').run(now(), machineId);
    if (rep.operation === 'backup') {
      enqueueEvent(orgId, `backup.${rep.status}`, {
        reportId: id, machineId, backupId: rep.backupId, backupName: rep.backupName, status: rep.status, finishedAt: rep.finishedAt, durationMs: rep.durationMs,
        filesExamined: rep.filesExamined, filesAdded: rep.filesAdded, filesModified: rep.filesModified, filesDeleted: rep.filesDeleted, bytesUploaded: rep.bytesUploaded,
        warnings: rep.warnings, errors: rep.errors, message: rep.message,
      });
    }
  })();
  return fromRow(d.prepare('SELECT * FROM backup_reports WHERE id = ?').get(id) as Row);
}

/** Keep the schedule the client reported in heartbeats (for missed-backup alerts). */
export function syncBackupSchedules(orgId: string, machineId: string, jobs: { id: string; name: string; scheduleIntervalMinutes: number | null }[]): void {
  const up = db().prepare(`INSERT INTO backups (org_id, machine_id, backup_id, name, schedule_interval_minutes) VALUES (?, ?, ?, ?, ?)
                           ON CONFLICT(machine_id, backup_id) DO UPDATE SET name = excluded.name, schedule_interval_minutes = excluded.schedule_interval_minutes`);
  db().transaction(() => {
    for (const j of jobs.slice(0, 200)) if (j.id) up.run(orgId, machineId, String(j.id).slice(0, 120), String(j.name ?? '').slice(0, 200), j.scheduleIntervalMinutes ?? null);
  })();
}

export interface BackupRollup {
  orgId: string;
  machineId: string;
  backupId: string;
  name: string;
  lastStatus: string | null;
  lastRunAt: string | null;
  lastSuccessAt: string | null;
  scheduleIntervalMinutes: number | null;
  sourceSize: number;
  backendSize: number;
  versionCount: number;
}

export function listBackups(orgIds: string[], machineIds?: string[]): BackupRollup[] {
  if (!orgIds.length || (machineIds && !machineIds.length)) return [];
  const where = [`org_id IN (${orgIds.map(() => '?').join(',')})`];
  const args: unknown[] = [...orgIds];
  if (machineIds) { where.push(`machine_id IN (${machineIds.map(() => '?').join(',')})`); args.push(...machineIds); }
  return (db().prepare(`SELECT * FROM backups WHERE ${where.join(' AND ')} ORDER BY lower(name)`).all(...args) as Row[]).map((r) => ({
    orgId: r.org_id as string,
    machineId: r.machine_id as string,
    backupId: r.backup_id as string,
    name: r.name as string,
    lastStatus: (r.last_status as string) ?? null,
    lastRunAt: (r.last_run_at as string) ?? null,
    lastSuccessAt: (r.last_success_at as string) ?? null,
    scheduleIntervalMinutes: (r.schedule_interval_minutes as number) ?? null,
    sourceSize: Number(r.source_size),
    backendSize: Number(r.backend_size),
    versionCount: Number(r.version_count),
  }));
}

export function listReports(o: { orgIds: string[]; machineId?: string; backupId?: string; status?: string; since?: string; limit?: number; machineIds?: string[] }): StoredReport[] {
  if (!o.orgIds.length || (o.machineIds && !o.machineIds.length)) return [];
  const where = [`org_id IN (${o.orgIds.map(() => '?').join(',')})`];
  const args: unknown[] = [...o.orgIds];
  if (o.machineIds) { where.push(`machine_id IN (${o.machineIds.map(() => '?').join(',')})`); args.push(...o.machineIds); }
  if (o.machineId) { where.push('machine_id = ?'); args.push(o.machineId); }
  if (o.backupId) { where.push('backup_id = ?'); args.push(o.backupId); }
  if (o.status) { where.push('status = ?'); args.push(o.status); }
  if (o.since) { where.push('finished_at >= ?'); args.push(o.since); }
  return (db().prepare(`SELECT * FROM backup_reports WHERE ${where.join(' AND ')} ORDER BY finished_at DESC LIMIT ?`).all(...args, Math.min(o.limit ?? 100, 5000)) as Row[]).map(fromRow);
}

export function reportRaw(id: string): unknown {
  const r = db().prepare('SELECT raw FROM backup_reports WHERE id = ?').get(id) as Row | undefined;
  return parseJson(r?.raw, null);
}

export interface StorageDay {
  day: string;
  backendSize: number;
  sourceSize: number;
  runs: number;
  failures: number;
}

/** Daily totals over the last `days` (latest size per backup per day, summed). */
export function storageHistory(orgIds: string[], days: number, machineIds?: string[]): StorageDay[] {
  const since = new Date(Date.now() - days * 86_400_000).toISOString();
  const reports = listReports({ orgIds, since, limit: 5000, machineIds }).filter((r) => r.operation === 'backup');
  const byDay = new Map<string, { sizes: Map<string, { b: number; s: number }>; runs: number; failures: number }>();
  for (const r of reports) {
    const day = r.finishedAt.slice(0, 10);
    const e = byDay.get(day) ?? { sizes: new Map(), runs: 0, failures: 0 };
    e.runs++;
    if (r.status === 'error') e.failures++;
    const key = `${r.machineId}/${r.backupId}`;
    if (!e.sizes.has(key) && (r.backendSize || r.sourceSize)) e.sizes.set(key, { b: r.backendSize, s: r.sourceSize });
    byDay.set(day, e);
  }
  return [...byDay.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([day, e]) => ({ day, backendSize: [...e.sizes.values()].reduce((a, x) => a + x.b, 0), sourceSize: [...e.sizes.values()].reduce((a, x) => a + x.s, 0), runs: e.runs, failures: e.failures }));
}
