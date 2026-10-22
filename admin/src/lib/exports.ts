/**
 * Background export jobs: large exports (the whole audit log, API request
 * log, backup reports, machines) are produced page by page off the request,
 * gzip-compressed into the database, and downloaded when ready. Progress is
 * visible while they run; files expire after EXPIRE_DAYS. Nothing is
 * truncated: every matching row is exported.
 */
import crypto from 'crypto';
import zlib from 'zlib';
import { db, newId, now, parseJson, type Row } from './db';
import { auditHead, listAudit, type AuditQuery } from './db/repos/audit';
import { listMachines } from './db/repos/machines';
import { managedSubtree, OrgError } from './db/repos/orgs';
import { exportAuditEntries } from './utils/audit-integrity';
import { renderReport } from './report-catalog';
import { AUDIT_EVENT_CATEGORIES, type AuditEntry } from './types/audit';
import type { Permission } from './types/auth';
import { logger } from './utils/logger';

export const EXPIRE_DAYS = 7;
const PAGE = 1000;

export type ExportKind = 'audit' | 'api_requests' | 'backup_reports' | 'machines';
export type ExportFormat = 'csv' | 'json' | 'syslog';
export type ExportStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'expired';

interface Source {
  label: string;
  permission: Permission;
  formats: ExportFormat[];
  /** The bound pages are read up to (stored as the `__until` parameter). */
  snapshot(): string;
  count(orgId: string, p: Record<string, string>): number;
  page(orgId: string, p: Record<string, string>, offset: number, limit: number): unknown[];
  /** One page in the format; `first`/`last` for headers and brackets. */
  render(rows: unknown[], format: ExportFormat, first: boolean, last: boolean): string;
}

/**
 * Pages are read oldest first up to a snapshot taken when the job starts
 * (`__until`: an audit sequence or a timestamp), so rows arriving during the
 * export neither shift pages nor appear twice.
 */
const auditQuery = (orgId: string, p: Record<string, string>): AuditQuery => ({
  orgIds: managedSubtree(orgId),
  untilSequence: Number(p.__until),
  ascending: true,
  eventType: p.eventType || undefined,
  eventTypes: p.category ? (AUDIT_EVENT_CATEGORIES[p.category] ?? []) : undefined,
  severity: p.severity || undefined,
  userId: p.userId || undefined,
  from: p.dateFrom || undefined,
  to: p.dateTo ? (p.dateTo.length === 10 ? `${p.dateTo}T23:59:59.999Z` : p.dateTo) : undefined,
  search: p.search || undefined,
});

/** Pages of a whole-document format joined into one: a CSV header once, one JSON array. */
function joinPage(text: string, format: ExportFormat, first: boolean, last: boolean, rows: number): string {
  if (format === 'csv') {
    const body = first ? text : text.slice(text.indexOf('\n') + 1);
    return rows ? `${body}\n` : first ? `${body}\n` : '';
  }
  if (format === 'json') {
    const inner = rows ? text.trim().replace(/^\[/, '').replace(/\]$/, '').trim() : '';
    return `${first ? '[\n' : inner ? ',\n' : ''}${inner}${last ? '\n]\n' : ''}`;
  }
  return rows ? `${text}\n` : '';
}

const resultFilter = (p: Record<string, string>) => (p.result === 'ok' ? ' AND status < 400' : p.result === 'refused' ? ' AND status >= 400' : '');

/** Tabular sources reuse the report renderer (CSV formula-safe). */
function tabular(columns: { key: string; label: string }[]) {
  return (rows: unknown[], format: ExportFormat, first: boolean, last: boolean) => {
    const r = { columns, rows: rows as Record<string, string | number | boolean | null>[], summary: '' };
    if (format === 'csv') return joinPage(renderReport(r, 'csv').replace(/\r\n/g, '\n').trimEnd(), 'csv', first, last, rows.length);
    return joinPage(JSON.stringify(rows), 'json', first, last, rows.length);
  };
}

export const SOURCES: Record<ExportKind, Source> = {
  audit: {
    label: 'Audit log',
    permission: 'audit.export',
    formats: ['csv', 'json', 'syslog'],
    snapshot: () => String(auditHead()),
    count: (orgId, p) => listAudit({ ...auditQuery(orgId, p), limit: 1 }).total,
    // Chain order.
    page: (orgId, p, offset, limit) => listAudit({ ...auditQuery(orgId, p), limit, offset }).entries,
    render: (rows, format, first, last) => joinPage(exportAuditEntries(rows as AuditEntry[], format), format, first, last, rows.length),
  },
  api_requests: {
    label: 'API request log',
    permission: 'audit.read',
    formats: ['csv', 'json'],
    snapshot: () => now(),
    count: (orgId, p) =>
      (
        db()
          .prepare(`SELECT COUNT(*) AS n FROM api_requests WHERE org_id = ? AND at <= ?${resultFilter(p)}`)
          .get(orgId, p.__until) as { n: number }
      ).n,
    page: (orgId, p, offset, limit) =>
      db()
        .prepare(
          `SELECT at, method, path, status, outcome, auth, token_id AS tokenId, user_email AS userEmail, ip, user_agent AS userAgent FROM api_requests WHERE org_id = ? AND at <= ?${resultFilter(p)} ORDER BY at, rowid LIMIT ? OFFSET ?`,
        )
        .all(orgId, p.__until, limit, offset) as Row[],
    render: tabular(['at', 'method', 'path', 'status', 'outcome', 'auth', 'tokenId', 'userEmail', 'ip', 'userAgent'].map((k) => ({ key: k, label: k }))),
  },
  backup_reports: {
    label: 'Backup reports',
    permission: 'fleet.read',
    formats: ['csv', 'json'],
    snapshot: () => now(),
    count: (orgId, p) =>
      (
        db()
          .prepare(`SELECT COUNT(*) AS n FROM backup_reports WHERE org_id = ? AND received_at <= ?${p.since ? ' AND finished_at >= ?' : ''}`)
          .get(orgId, p.__until, ...(p.since ? [p.since] : [])) as { n: number }
      ).n,
    page: (orgId, p, offset, limit) =>
      db()
        .prepare(
          `SELECT id, machine_id, backup_id, backup_name, operation, status, started_at, finished_at, duration_ms, files_examined, bytes_uploaded, source_size, backend_size, warnings, errors, message FROM backup_reports WHERE org_id = ? AND received_at <= ?${p.since ? ' AND finished_at >= ?' : ''} ORDER BY received_at, rowid LIMIT ? OFFSET ?`,
        )
        .all(orgId, p.__until, ...(p.since ? [p.since] : []), limit, offset) as Row[],
    render: tabular(
      [
        'id',
        'machine_id',
        'backup_id',
        'backup_name',
        'operation',
        'status',
        'started_at',
        'finished_at',
        'duration_ms',
        'files_examined',
        'bytes_uploaded',
        'source_size',
        'backend_size',
        'warnings',
        'errors',
        'message',
      ].map((k) => ({ key: k, label: k })),
    ),
  },
  machines: {
    label: 'Machines',
    permission: 'fleet.read',
    formats: ['csv', 'json'],
    snapshot: () => now(),
    count: (orgId) => listMachines({ orgIds: [orgId] }).length,
    page: (orgId, _p, offset, limit) =>
      listMachines({ orgIds: [orgId] })
        .slice(offset, offset + limit)
        .map((m) => ({
          id: m.id,
          name: m.name,
          hostname: m.hostname,
          os: m.os,
          osVersion: m.osVersion,
          clientVersion: m.clientVersion,
          status: m.status,
          connectivity: m.connectivity,
          lastSeenAt: m.lastSeenAt,
          lastIp: m.lastIp,
          tags: m.tags.join(' '),
          registeredAt: m.registeredAt,
        })),
    render: tabular(['id', 'name', 'hostname', 'os', 'osVersion', 'clientVersion', 'status', 'connectivity', 'lastSeenAt', 'lastIp', 'tags', 'registeredAt'].map((k) => ({ key: k, label: k }))),
  },
};

export interface ExportJob {
  id: string;
  orgId: string;
  kind: ExportKind;
  format: ExportFormat;
  params: Record<string, string>;
  status: ExportStatus;
  total: number;
  done: number;
  size: number;
  sha256: string | null;
  error: string | null;
  requestedBy: string;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  expiresAt: string | null;
}

function fromRow(r: Row): ExportJob {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    kind: r.kind as ExportKind,
    format: r.format as ExportFormat,
    params: parseJson(r.params, {}),
    status: r.status as ExportStatus,
    total: Number(r.total),
    done: Number(r.done),
    size: Number(r.size),
    sha256: (r.sha256 as string) ?? null,
    error: (r.error as string) ?? null,
    requestedBy: r.requested_by as string,
    createdAt: r.created_at as string,
    startedAt: (r.started_at as string) ?? null,
    finishedAt: (r.finished_at as string) ?? null,
    expiresAt: (r.expires_at as string) ?? null,
  };
}

const COLS = 'id, org_id, kind, format, params, status, total, done, size, sha256, error, requested_by, created_at, started_at, finished_at, expires_at';

export function queueExport(orgId: string, kind: unknown, format: unknown, params: unknown, by: string): ExportJob {
  const src = SOURCES[kind as ExportKind];
  if (!src) throw new OrgError('Unknown export');
  if (!src.formats.includes(format as ExportFormat)) throw new OrgError(`${src.label} exports as ${src.formats.join(', ')}`);
  const p = Object.fromEntries(
    Object.entries((params && typeof params === 'object' ? params : {}) as Record<string, unknown>)
      .filter(([k, v]) => typeof v === 'string' && v && !k.startsWith('__'))
      .map(([k, v]) => [k, String(v).slice(0, 200)]),
  );
  const active = (db().prepare("SELECT COUNT(*) AS n FROM export_jobs WHERE org_id = ? AND status IN ('queued', 'running')").get(orgId) as { n: number }).n;
  if (active >= 5) throw new OrgError('Five exports are already in progress for this organization; wait for one to finish', 429);
  const id = newId('exp');
  db().prepare("INSERT INTO export_jobs (id, org_id, kind, format, params, status, requested_by, created_at) VALUES (?, ?, ?, ?, ?, 'queued', ?, ?)").run(id, orgId, kind, format, JSON.stringify(p), by, now());
  setTimeout(() => void processExports(), 0).unref?.();
  return getExport(orgId, id)!;
}

export function getExport(orgId: string, id: string): ExportJob | null {
  const r = db().prepare(`SELECT ${COLS} FROM export_jobs WHERE org_id = ? AND id = ?`).get(orgId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function listExports(orgId: string, requestedBy?: string): ExportJob[] {
  const rows = requestedBy
    ? db().prepare(`SELECT ${COLS} FROM export_jobs WHERE org_id = ? AND requested_by = ? ORDER BY created_at DESC, rowid DESC LIMIT 50`).all(orgId, requestedBy)
    : db().prepare(`SELECT ${COLS} FROM export_jobs WHERE org_id = ? ORDER BY created_at DESC, rowid DESC LIMIT 50`).all(orgId);
  return (rows as Row[]).map(fromRow);
}

/** The finished file (gzip). */
export function exportFile(orgId: string, id: string): Buffer | null {
  const r = db().prepare("SELECT file FROM export_jobs WHERE org_id = ? AND id = ? AND status = 'succeeded'").get(orgId, id) as Row | undefined;
  return r?.file ? Buffer.from(r.file as Uint8Array) : null;
}

export function deleteExport(orgId: string, id: string): boolean {
  return db().prepare("DELETE FROM export_jobs WHERE org_id = ? AND id = ? AND status != 'running'").run(orgId, id).changes > 0;
}

async function runJob(job: ExportJob): Promise<void> {
  const src = SOURCES[job.kind];
  const params = { ...job.params, __until: src.snapshot() };
  job = { ...job, params };
  const total = src.count(job.orgId, job.params);
  db().prepare("UPDATE export_jobs SET status = 'running', total = ?, started_at = ? WHERE id = ?").run(total, now(), job.id);
  const gzip = zlib.createGzip();
  const chunks: Buffer[] = [];
  const hash = crypto.createHash('sha256');
  gzip.on('data', (c: Buffer) => chunks.push(c));
  const finished = new Promise<void>((resolve, reject) => {
    gzip.on('end', resolve);
    gzip.on('error', reject);
  });
  let done = 0;
  for (let offset = 0; offset < total || offset === 0; offset += PAGE) {
    const rows = total ? src.page(job.orgId, job.params, offset, Math.min(PAGE, total - offset)) : [];
    const text = src.render(rows, job.format, offset === 0, offset + PAGE >= total);
    hash.update(text);
    gzip.write(text);
    done += rows.length;
    db().prepare('UPDATE export_jobs SET done = ? WHERE id = ?').run(done, job.id);
    // Let requests through between pages.
    await new Promise((r) => setImmediate(r));
    if (!total) break;
  }
  gzip.end();
  await finished;
  const file = Buffer.concat(chunks);
  db()
    .prepare("UPDATE export_jobs SET status = 'succeeded', file = ?, size = ?, sha256 = ?, finished_at = ?, expires_at = ? WHERE id = ?")
    .run(file, file.length, hash.digest('hex'), now(), new Date(Date.now() + EXPIRE_DAYS * 86_400_000).toISOString(), job.id);
}

let running: Promise<number> | null = null;

/** Work through queued exports one at a time; expire old files. Returns how many ran. */
export function processExports(at = new Date()): Promise<number> {
  if (running) return running;
  running = (async () => {
    db().prepare("UPDATE export_jobs SET status = 'expired', file = NULL WHERE status = 'succeeded' AND expires_at < ?").run(at.toISOString());
    // A job left running by a restart is retried.
    db()
      .prepare("UPDATE export_jobs SET status = 'queued', done = 0 WHERE status = 'running' AND started_at < ?")
      .run(new Date(at.getTime() - 3600_000).toISOString());
    let n = 0;
    for (;;) {
      const r = db().prepare(`SELECT ${COLS} FROM export_jobs WHERE status = 'queued' ORDER BY created_at, rowid LIMIT 1`).get() as Row | undefined;
      if (!r) break;
      const job = fromRow(r);
      try {
        await runJob(job);
      } catch (e) {
        const error = e instanceof Error ? e.message : String(e);
        logger.error('exports.failed', { id: job.id, error });
        db().prepare("UPDATE export_jobs SET status = 'failed', error = ?, finished_at = ? WHERE id = ?").run(error.slice(0, 500), now(), job.id);
      }
      n++;
    }
    return n;
  })().finally(() => {
    running = null;
  });
  return running;
}
