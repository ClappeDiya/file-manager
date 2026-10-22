/**
 * Running catalog reports (`report-catalog.ts`): on demand or on a schedule
 * (daily / weekly / monthly at a local hour in a time zone), with every run
 * recorded (status, row count, summary, the result for download, delivery
 * outcome per channel). Scheduled runs are sent through the notification
 * channels: email gets the CSV attached (up to 1 MB) and a text table; chat
 * and webhooks get the summary and the run id.
 */
import { effectiveBranding, renderTemplate } from './branding';
import { db, newId, now, parseJson, type Row } from './db';
import { OrgError, getOrg } from './db/repos/orgs';
import { getChannel } from './db/repos/channels';
import { appendAudit } from './db/repos/audit';
import { deliverTo, type Delivery } from './mailer';
import { isTimeZone, nextOccurrence, type Recurrence } from './zoned-time';
import { renderReport, reportDefinition, REPORTS, type ReportFormat, type ReportResult } from './report-catalog';
import { logger } from './utils/logger';

export const MAX_ATTACHMENT_BYTES = 1_000_000;
/** Runs kept per organization. */
const KEEP_RUNS = 500;

export type RunStatus = 'running' | 'succeeded' | 'failed';

export interface ReportRun {
  id: string;
  orgId: string;
  reportId: string;
  scheduleId: string | null;
  days: number;
  status: RunStatus;
  rowCount: number;
  summary: string;
  error: string | null;
  delivery: { channelId: string; ok: boolean; error?: string }[];
  requestedBy: string;
  startedAt: string;
  finishedAt: string | null;
}

function runFromRow(r: Row): ReportRun {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    reportId: r.report_id as string,
    scheduleId: (r.schedule_id as string) ?? null,
    days: Number(r.days),
    status: r.status as RunStatus,
    rowCount: Number(r.row_count),
    summary: r.summary as string,
    error: (r.error as string) ?? null,
    delivery: parseJson(r.delivery, []),
    requestedBy: r.requested_by as string,
    startedAt: r.started_at as string,
    finishedAt: (r.finished_at as string) ?? null,
  };
}

export function clampDays(days: unknown, fallback: number): number {
  const n = Number(days ?? fallback);
  return Number.isInteger(n) && n >= 1 && n <= 365 ? n : fallback;
}

/** Run a report now and record the run (the result is kept for download). */
export function runReport(orgId: string, reportId: string, o: { days?: number; by: string; scheduleId?: string | null }): { run: ReportRun; result: ReportResult | null } {
  const def = reportDefinition(reportId);
  if (!def) throw new OrgError(`Unknown report ${reportId}`, 404);
  const days = clampDays(o.days, def.defaultDays);
  const id = newId('rrn');
  db()
    .prepare("INSERT INTO report_runs (id, org_id, report_id, schedule_id, days, status, requested_by, started_at) VALUES (?, ?, ?, ?, ?, 'running', ?, ?)")
    .run(id, orgId, reportId, o.scheduleId ?? null, days, o.by, now());
  let result: ReportResult | null = null;
  try {
    result = def.run(orgId, days);
    db().prepare("UPDATE report_runs SET status = 'succeeded', row_count = ?, summary = ?, result = ?, finished_at = ? WHERE id = ?").run(result.rows.length, result.summary, JSON.stringify(result), now(), id);
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e);
    logger.error('reports.run_failed', { reportId, error });
    db().prepare("UPDATE report_runs SET status = 'failed', error = ?, finished_at = ? WHERE id = ?").run(error.slice(0, 500), now(), id);
  }
  db().prepare('DELETE FROM report_runs WHERE org_id = ? AND id NOT IN (SELECT id FROM report_runs WHERE org_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?)').run(orgId, orgId, KEEP_RUNS);
  return { run: getRun(orgId, id)!, result };
}

export function getRun(orgId: string, id: string): ReportRun | null {
  const r = db().prepare('SELECT * FROM report_runs WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? runFromRow(r) : null;
}

/** The stored result of a successful run. */
export function runResult(orgId: string, id: string): ReportResult | null {
  const r = db().prepare("SELECT result FROM report_runs WHERE org_id = ? AND id = ? AND status = 'succeeded'").get(orgId, id) as Row | undefined;
  return r ? parseJson<ReportResult | null>(r.result, null) : null;
}

export function listRuns(orgId: string, o: { scheduleId?: string; limit?: number } = {}): ReportRun[] {
  const rows = o.scheduleId
    ? db()
        .prepare('SELECT * FROM report_runs WHERE org_id = ? AND schedule_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?')
        .all(orgId, o.scheduleId, Math.min(o.limit ?? 50, 200))
    : db()
        .prepare('SELECT * FROM report_runs WHERE org_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?')
        .all(orgId, Math.min(o.limit ?? 50, 200));
  return (rows as Row[]).map(runFromRow);
}

// ── Schedules ──

export interface ReportSchedule extends Recurrence {
  id: string;
  orgId: string;
  name: string;
  reportId: string;
  days: number;
  /** Attachment format for email. */
  format: Exclude<ReportFormat, 'text'>;
  channelIds: string[];
  enabled: boolean;
  nextRunAt: string | null;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
}

function scheduleFromRow(r: Row): ReportSchedule {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    reportId: r.report_id as string,
    days: Number(r.days),
    format: r.format as ReportSchedule['format'],
    frequency: r.frequency as Recurrence['frequency'],
    hour: Number(r.hour),
    weekday: r.weekday === null ? undefined : Number(r.weekday),
    dayOfMonth: r.day_of_month === null ? undefined : Number(r.day_of_month),
    timeZone: r.time_zone as string,
    channelIds: parseJson<string[]>(r.channel_ids, []),
    enabled: !!r.enabled,
    nextRunAt: (r.next_run_at as string) ?? null,
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    updatedAt: r.updated_at as string,
  };
}

export interface ScheduleInput {
  name?: string;
  reportId: string;
  days?: number;
  format?: 'csv' | 'json';
  frequency: Recurrence['frequency'];
  hour: number;
  weekday?: number;
  dayOfMonth?: number;
  timeZone?: string;
  channelIds: string[];
  enabled?: boolean;
}

function validate(orgId: string, i: ScheduleInput): Omit<ReportSchedule, 'id' | 'orgId' | 'nextRunAt' | 'createdBy' | 'createdAt' | 'updatedAt'> {
  const def = reportDefinition(i.reportId);
  if (!def) throw new OrgError(`Unknown report ${i.reportId}`);
  if (!['daily', 'weekly', 'monthly'].includes(i.frequency)) throw new OrgError('The frequency is daily, weekly or monthly');
  if (!Number.isInteger(i.hour) || i.hour < 0 || i.hour > 23) throw new OrgError('The hour is 0 to 23');
  if (i.frequency === 'weekly' && !(Number.isInteger(i.weekday) && i.weekday! >= 0 && i.weekday! <= 6)) throw new OrgError('Choose the day of the week');
  if (i.frequency === 'monthly' && !(Number.isInteger(i.dayOfMonth) && i.dayOfMonth! >= 1 && i.dayOfMonth! <= 28)) throw new OrgError('The day of the month is 1 to 28');
  const timeZone = i.timeZone || 'UTC';
  if (!isTimeZone(timeZone)) throw new OrgError('Unknown time zone');
  if (!Array.isArray(i.channelIds) || !i.channelIds.length) throw new OrgError('Choose at least one channel to send it to');
  for (const c of i.channelIds) if (!getChannel(orgId, c)) throw new OrgError(`Channel ${c} not found`);
  const format = i.format ?? 'csv';
  if (!['csv', 'json'].includes(format)) throw new OrgError('The attachment format is csv or json');
  return {
    name: i.name?.trim() || def.name,
    reportId: def.id,
    days: clampDays(i.days, def.defaultDays),
    format,
    frequency: i.frequency,
    hour: i.hour,
    weekday: i.frequency === 'weekly' ? i.weekday : undefined,
    dayOfMonth: i.frequency === 'monthly' ? i.dayOfMonth : undefined,
    timeZone,
    channelIds: [...new Set(i.channelIds)],
    enabled: i.enabled !== false,
  };
}

export function createSchedule(orgId: string, i: ScheduleInput, by: string, at = new Date()): ReportSchedule {
  const v = validate(orgId, i);
  const id = newId('rsc');
  db()
    .prepare(
      'INSERT INTO report_schedules (id, org_id, name, report_id, days, format, frequency, hour, weekday, day_of_month, time_zone, channel_ids, enabled, next_run_at, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
    )
    .run(
      id,
      orgId,
      v.name,
      v.reportId,
      v.days,
      v.format,
      v.frequency,
      v.hour,
      v.weekday ?? null,
      v.dayOfMonth ?? null,
      v.timeZone,
      JSON.stringify(v.channelIds),
      v.enabled ? 1 : 0,
      v.enabled ? nextOccurrence(at, v).toISOString() : null,
      by,
      now(),
      now(),
    );
  return getSchedule(orgId, id)!;
}

export function updateSchedule(orgId: string, id: string, i: Partial<ScheduleInput>, at = new Date()): ReportSchedule | null {
  const cur = getSchedule(orgId, id);
  if (!cur) return null;
  const v = validate(orgId, { ...cur, ...i } as ScheduleInput);
  db()
    .prepare(
      'UPDATE report_schedules SET name = ?, report_id = ?, days = ?, format = ?, frequency = ?, hour = ?, weekday = ?, day_of_month = ?, time_zone = ?, channel_ids = ?, enabled = ?, next_run_at = ?, updated_at = ? WHERE id = ?',
    )
    .run(
      v.name,
      v.reportId,
      v.days,
      v.format,
      v.frequency,
      v.hour,
      v.weekday ?? null,
      v.dayOfMonth ?? null,
      v.timeZone,
      JSON.stringify(v.channelIds),
      v.enabled ? 1 : 0,
      v.enabled ? nextOccurrence(at, v).toISOString() : null,
      now(),
      id,
    );
  return getSchedule(orgId, id);
}

export function getSchedule(orgId: string, id: string): ReportSchedule | null {
  const r = db().prepare('SELECT * FROM report_schedules WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? scheduleFromRow(r) : null;
}

export function listSchedules(orgId: string): ReportSchedule[] {
  return (db().prepare('SELECT * FROM report_schedules WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(scheduleFromRow);
}

export function deleteSchedule(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM report_schedules WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

/** Run a schedule now and send the result to its channels. */
export async function runSchedule(s: ReportSchedule, by: string): Promise<ReportRun> {
  const { run, result } = runReport(s.orgId, s.reportId, { days: s.days, by, scheduleId: s.id });
  const def = reportDefinition(s.reportId)!;
  const org = getOrg(s.orgId);
  let delivery: Delivery[];
  if (result) {
    const file = renderReport(result, s.format);
    const attach = Buffer.byteLength(file) <= MAX_ATTACHMENT_BYTES;
    delivery = await deliverTo(
      s.orgId,
      s.channelIds,
      {
        ...renderTemplate(s.orgId, 'report', {
          report: s.name,
          organization: org?.name ?? s.orgId,
          body: `${renderReport(result, 'text')}${attach ? '' : `\n\nThe full report (${result.rows.length} rows) is too large to attach; download run ${run.id} from Reports.`}`,
        }),
        data: { event: 'report.run', reportId: def.id, runId: run.id, scheduleId: s.id, rows: result.rows.length, summary: result.summary },
        attachments: attach ? [{ filename: `${def.id}-${run.startedAt.slice(0, 10)}.${s.format}`, content: file, contentType: s.format === 'csv' ? 'text/csv' : 'application/json' }] : undefined,
      },
      'report',
    );
  } else {
    delivery = await deliverTo(
      s.orgId,
      s.channelIds,
      {
        subject: `[${effectiveBranding(s.orgId).displayName}] ${s.name} failed — ${org?.name ?? s.orgId}`,
        text: `The scheduled report could not be produced: ${run.error}`,
        data: { event: 'report.failed', reportId: def.id, runId: run.id, scheduleId: s.id },
      },
      'report',
    );
  }
  const outcome = delivery.map((d) => ({ channelId: d.channelId, ok: d.ok, ...(d.error ? { error: d.error } : {}) }));
  db().prepare('UPDATE report_runs SET delivery = ? WHERE id = ?').run(JSON.stringify(outcome), run.id);
  appendAudit({
    eventType: 'report.sent',
    orgId: s.orgId,
    userEmail: by,
    severity: run.status === 'failed' || outcome.some((d) => !d.ok) ? 'warning' : 'info',
    description: `Report "${s.name}" ${run.status} (${run.rowCount} rows), sent to ${outcome.filter((d) => d.ok).length}/${outcome.length} channel(s)`,
    resourceType: 'report_schedule',
    resourceId: s.id,
    metadata: { runId: run.id },
  });
  return getRun(s.orgId, run.id)!;
}

/** Periodic job: run every due schedule once and plan its next run. */
export async function runDueSchedules(at = new Date()): Promise<number> {
  const due = (db().prepare('SELECT * FROM report_schedules WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?').all(at.toISOString()) as Row[]).map(scheduleFromRow);
  for (const s of due) {
    // Plan the next run first, so a crash mid-run does not repeat it in a loop.
    db().prepare('UPDATE report_schedules SET next_run_at = ? WHERE id = ?').run(nextOccurrence(at, s).toISOString(), s.id);
    await runSchedule(s, 'scheduler');
  }
  return due.length;
}

export const CATALOG = () => REPORTS.map(({ id, name, description, category, permission, defaultDays }) => ({ id, name, description, category, permission, defaultDays }));
