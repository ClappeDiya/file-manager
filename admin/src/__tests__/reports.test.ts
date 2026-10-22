/** Report catalog: every report over seeded data, downloads, schedules (time zones) and emailed runs with attachments. */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import net from 'net';
import { createOrg } from '@/lib/db/repos/orgs';
import { createLink, registerWithToken } from '@/lib/db/repos/machines';
import { ingestReport, normalizeReport } from '@/lib/db/repos/reports';
import { createChannel } from '@/lib/db/repos/channels';
import { appendAudit, listAudit } from '@/lib/db/repos/audit';
import { REPORTS, renderReport } from '@/lib/report-catalog';
import { getSchedule, runDueSchedules } from '@/lib/report-runner';
import { nextOccurrence } from '@/lib/zoned-time';
import { json, makeUser, req } from './helpers';
import * as reports from '@/app/api/reports/route';
import * as run from '@/app/api/reports/run/route';
import * as runById from '@/app/api/reports/runs/[id]/route';
import * as schedules from '@/app/api/reports/schedules/route';
import * as scheduleById from '@/app/api/reports/schedules/[id]/route';
import * as runNow from '@/app/api/reports/schedules/[id]/run/route';

// A minimal SMTP receiver (no TLS, no auth) that keeps each message's DATA.
let smtp: net.Server;
let smtpPort = 0;
const mails: string[] = [];
beforeAll(async () => {
  smtp = net.createServer((s) => {
    let data = false;
    let buf = '';
    s.write('220 test ESMTP\r\n');
    s.on('data', (chunk) => {
      buf += chunk.toString();
      let i: number;
      while (!data && (i = buf.indexOf('\r\n')) >= 0) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 2);
        const cmd = line.slice(0, 4).toUpperCase();
        if (cmd === 'EHLO' || cmd === 'HELO') s.write('250 test\r\n');
        else if (cmd === 'DATA') {
          data = true;
          s.write('354 go\r\n');
        } else if (cmd === 'QUIT') s.end('221 bye\r\n');
        else s.write('250 ok\r\n');
      }
      if (data && (i = buf.indexOf('\r\n.\r\n')) >= 0) {
        mails.push(buf.slice(0, i));
        buf = buf.slice(i + 5);
        data = false;
        s.write('250 queued\r\n');
      }
    });
  });
  await new Promise<void>((r) => smtp.listen(0, '127.0.0.1', r));
  smtpPort = (smtp.address() as net.AddressInfo).port;
});
afterAll(() => smtp.close());

function seed(orgId: string) {
  const { token } = createLink(orgId, {}, 't');
  const { machine } = registerWithToken(token, { name: 'ledger-pc', os: 'windows' }, ['finance']);
  ingestReport(orgId, machine.id, normalizeReport({ backup_id: 'b1', name: 'Ledger', result: 'success', stats: { backend_size: 1000 } }), {});
  ingestReport(orgId, machine.id, normalizeReport({ backup_id: 'b1', name: 'Ledger', result: 'error', message: '=cmd|calc Access denied', stats: { backend_size: 1500 } }), {});
  appendAudit({ eventType: 'machine.register', orgId, description: 'x' });
  return machine;
}

describe('report catalog', () => {
  it('runs every report over real data and renders CSV, JSON and text', async () => {
    const org = createOrg({ name: 'Reports org' });
    const admin = makeUser('rep-admin@x.org', 'org_admin', org.id);
    seed(org.id);
    for (const def of REPORTS) {
      const r = await json<{ run: { id: string; status: string; rowCount: number; summary: string }; result: { columns: unknown[]; rows: Record<string, unknown>[] } }>(
        await run.POST(await req('POST', '/api/reports/run', { as: admin, body: { reportId: def.id } })),
      );
      expect(r.status, def.id).toBe(201);
      expect(r.body.run).toMatchObject({ status: 'succeeded' });
      expect(r.body.run.rowCount).toBe(r.body.result.rows.length);
    }
    const status = await json<{ run: { id: string }; result: { rows: Record<string, unknown>[] } }>(await run.POST(await req('POST', '/api/reports/run', { as: admin, body: { reportId: 'backup_status', days: 7 } })));
    expect(status.body.result.rows).toEqual([expect.objectContaining({ machine: 'ledger-pc', backup: 'Ledger', lastStatus: 'error', runs: 2, successRate: 50, backendSize: 1500 })]);
    const failed = await json<{ run: { id: string } }>(await run.POST(await req('POST', '/api/reports/run', { as: admin, body: { reportId: 'failed_backups' } })));
    const csv = await (await runById.GET(await req('GET', `/api/reports/runs/${failed.body.run.id}?format=csv`, { as: admin }), { params: Promise.resolve({ id: failed.body.run.id }) })).text();
    expect(csv.split('\r\n')[0]).toBe('Finished,Machine,Backup,Operation,Errors,Message');
    expect(csv).toContain(`'=cmd|calc Access denied`); // formula neutralized
    const growth = await json<{ result: { rows: { change: number }[] } }>(await run.POST(await req('POST', '/api/reports/run', { as: admin, body: { reportId: 'storage_growth' } })));
    expect(growth.body.result.rows[0].change).toBe(500);
    const access = await json<{ result: { rows: Record<string, unknown>[] } }>(await run.POST(await req('POST', '/api/reports/run', { as: admin, body: { reportId: 'access_review' } })));
    expect(access.body.result.rows).toEqual([expect.objectContaining({ email: 'rep-admin@x.org', role: 'org_admin', twoFactor: false, accountActive: true })]);

    const text = renderReport({ columns: [{ key: 'a', label: 'A' }], rows: Array.from({ length: 60 }, (_, i) => ({ a: i })), summary: 'sum' }, 'text');
    expect(text).toContain('… and 10 more row(s)');
    expect((await runById.GET(await req('GET', `/api/reports/runs/${failed.body.run.id}?format=xml`, { as: admin }), { params: Promise.resolve({ id: failed.body.run.id }) })).status).toBe(400);
  });

  it('only offers and runs the reports a role may see', async () => {
    const org = createOrg({ name: 'Reports roles' });
    const approver = makeUser('rep-approver@x.org', 'approver', org.id);
    const list = await json<{ catalog: { id: string }[] }>(await reports.GET(await req('GET', '/api/reports', { as: approver })));
    expect(list.body.catalog.map((r) => r.id).sort()).toEqual(['audit_summary', 'backup_status', 'failed_backups', 'machine_inventory', 'stale_machines', 'storage_growth']);
    expect((await run.POST(await req('POST', '/api/reports/run', { as: approver, body: { reportId: 'access_review' } }))).status).toBe(403);
    expect((await run.POST(await req('POST', '/api/reports/run', { as: approver, body: { reportId: 'nope' } }))).status).toBe(404);
    const other = makeUser('rep-other@x.org', 'org_admin', createOrg({ name: 'Rep other' }).id);
    const mine = await json<{ run: { id: string } }>(await run.POST(await req('POST', '/api/reports/run', { as: approver, body: { reportId: 'audit_summary' } })));
    expect((await runById.GET(await req('GET', `/api/reports/runs/${mine.body.run.id}?format=csv`, { as: other }), { params: Promise.resolve({ id: mine.body.run.id }) })).status).toBe(404);
  });

  it('computes the next run in the schedule time zone across DST', () => {
    const tz = 'Europe/Berlin';
    // Before the spring change (Sunday 2026-03-29): 08:00 local is 07:00 UTC, after it 06:00 UTC.
    expect(nextOccurrence(new Date('2026-03-28T08:00:00Z'), { frequency: 'daily', hour: 8, timeZone: tz }).toISOString()).toBe('2026-03-29T06:00:00.000Z');
    expect(nextOccurrence(new Date('2026-03-27T06:00:00Z'), { frequency: 'daily', hour: 8, timeZone: tz }).toISOString()).toBe('2026-03-27T07:00:00.000Z');
    expect(nextOccurrence(new Date('2026-03-27T12:00:00Z'), { frequency: 'weekly', weekday: 1, hour: 9, timeZone: 'America/New_York' }).toISOString()).toBe('2026-03-30T13:00:00.000Z');
    expect(nextOccurrence(new Date('2026-03-27T12:00:00Z'), { frequency: 'monthly', dayOfMonth: 1, hour: 0, timeZone: 'UTC' }).toISOString()).toBe('2026-04-01T00:00:00.000Z');
  });

  it('schedules a report, emails it with the CSV attached and records the run and delivery', async () => {
    const org = createOrg({ name: 'Scheduled org' });
    const admin = makeUser('sched-admin@x.org', 'org_admin', org.id);
    seed(org.id);
    const mail = createChannel(org.id, { name: 'Ops mail', kind: 'email', config: { host: '127.0.0.1', port: smtpPort, security: 'none', username: '', from: 'ufop@x.org', to: ['ops@x.org'] } });
    const bad = await json<{ error: string }>(await schedules.POST(await req('POST', '/api/reports/schedules', { as: admin, body: { reportId: 'backup_status', frequency: 'weekly', hour: 7, channelIds: [mail.id] } })));
    expect(bad.body.error).toMatch(/day of the week/);
    expect((await schedules.POST(await req('POST', '/api/reports/schedules', { as: admin, body: { reportId: 'backup_status', frequency: 'daily', hour: 7, channelIds: [] } }))).status).toBe(400);
    const created = await json<{ schedule: { id: string; nextRunAt: string; name: string } }>(
      await schedules.POST(await req('POST', '/api/reports/schedules', { as: admin, body: { reportId: 'backup_status', frequency: 'daily', hour: 7, timeZone: 'Asia/Tokyo', channelIds: [mail.id] } })),
    );
    expect(created.status).toBe(201);
    expect(created.body.schedule.name).toBe('Backup status');
    expect(new Date(created.body.schedule.nextRunAt).getUTCHours()).toBe(22); // 07:00 in Tokyo

    // Not due yet; then due.
    expect(await runDueSchedules(new Date(Date.parse(created.body.schedule.nextRunAt) - 60_000))).toBe(0);
    expect(await runDueSchedules(new Date(Date.parse(created.body.schedule.nextRunAt) + 1000))).toBe(1);
    expect(mails).toHaveLength(1);
    const m = mails[0];
    expect(m).toMatch(/Subject: \[UFOP\] Backup status =?.*Scheduled org|Subject: =\?UTF-8/);
    expect(m).toContain('ledger-pc');
    expect(m).toMatch(/filename=.?backup_status-\d{4}-\d{2}-\d{2}\.csv/);
    const next = getSchedule(org.id, created.body.schedule.id)!.nextRunAt!;
    expect(Date.parse(next) - Date.parse(created.body.schedule.nextRunAt)).toBe(86_400_000);

    const detail = await json<{ runs: { status: string; delivery: { ok: boolean }[]; requestedBy: string }[] }>(
      await scheduleById.GET(await req('GET', `/api/reports/schedules/${created.body.schedule.id}`, { as: admin }), { params: Promise.resolve({ id: created.body.schedule.id }) }),
    );
    expect(detail.body.runs[0]).toMatchObject({ status: 'succeeded', requestedBy: 'scheduler', delivery: [{ ok: true }] });
    expect(listAudit({ eventType: 'report.sent' }).entries[0].description).toContain('sent to 1/1 channel(s)');

    // Run now; pausing clears the next run.
    expect((await runNow.POST(await req('POST', `/api/reports/schedules/${created.body.schedule.id}/run`, { as: admin }), { params: Promise.resolve({ id: created.body.schedule.id }) })).status).toBe(201);
    expect(mails).toHaveLength(2);
    const paused = await json<{ schedule: { enabled: boolean; nextRunAt: string | null } }>(
      await scheduleById.PATCH(await req('PATCH', `/api/reports/schedules/${created.body.schedule.id}`, { as: admin, body: { enabled: false } }), { params: Promise.resolve({ id: created.body.schedule.id }) }),
    );
    expect(paused.body.schedule).toMatchObject({ enabled: false, nextRunAt: null });
    expect((await scheduleById.DELETE(await req('DELETE', `/api/reports/schedules/${created.body.schedule.id}`, { as: admin }), { params: Promise.resolve({ id: created.body.schedule.id }) })).status).toBe(200);
  });
});
