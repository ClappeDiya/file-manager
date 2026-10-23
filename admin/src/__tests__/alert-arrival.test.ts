/**
 * Expected-arrival alerts (deadline in a time zone, chosen days, grace) and
 * acknowledging / snoozing alerts so they stop notifying.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import http from 'http';
import type { AddressInfo } from 'net';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { createLink, registerWithToken } from '@/lib/db/repos/machines';
import { createChannel } from '@/lib/db/repos/channels';
import { createRule, getAlert, listAlerts } from '@/lib/db/repos/alerts';
import { ingestReport, normalizeReport } from '@/lib/db/repos/reports';
import { evaluateMissed, evaluateReport } from '@/lib/alerts';
import { getMachine } from '@/lib/db/repos/machines';
import { localParts, recentDeadlines, zonedToUtc } from '@/lib/zoned-time';
import { json, makeUser, req } from './helpers';
import * as alertById from '@/app/api/alerts/[id]/route';
import * as rules from '@/app/api/alerts/rules/route';

const P = (id: string) => ({ params: Promise.resolve({ id }) });
let hits = 0;
let server: http.Server;
let url = '';
beforeAll(async () => {
  server = http.createServer((rq, rs) => {
    rq.resume();
    rq.on('end', () => {
      hits++;
      rs.end('ok');
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/h`;
});
afterAll(() => server.close());

describe('zoned time', () => {
  it('converts local wall-clock times across DST', () => {
    // New York: EST (UTC-5) in January, EDT (UTC-4) after 8 March 2026.
    expect(zonedToUtc(2026, 1, 15, 7, 0, 'America/New_York').toISOString()).toBe('2026-01-15T12:00:00.000Z');
    expect(zonedToUtc(2026, 3, 9, 7, 0, 'America/New_York').toISOString()).toBe('2026-03-09T11:00:00.000Z');
    expect(zonedToUtc(2026, 7, 1, 23, 30, 'Asia/Kolkata').toISOString()).toBe('2026-07-01T18:00:00.000Z');
    expect(localParts(new Date('2026-10-25T00:30:00Z'), 'Europe/Berlin')).toMatchObject({ y: 2026, m: 10, d: 25, hh: 2, mm: 30, weekday: 0 });
  });

  it('finds the latest deadlines on chosen days', () => {
    // Friday 2026-10-23 10:00 UTC; weekdays at 07:00 Berlin (05:00 UTC in summer time).
    const at = new Date('2026-10-23T10:00:00Z');
    const d = recentDeadlines(at, { expectedBy: '07:00', days: [1, 2, 3, 4, 5], timeZone: 'Europe/Berlin' });
    expect(d.map((x) => x.toISOString())).toEqual(['2026-10-23T05:00:00.000Z', '2026-10-22T05:00:00.000Z']);
    // Monday morning before the deadline: the last two are Friday and Thursday.
    const mon = recentDeadlines(new Date('2026-10-26T05:00:00Z'), { expectedBy: '07:00', days: [1, 2, 3, 4, 5], timeZone: 'Europe/Berlin' });
    expect(mon.map((x) => x.toISOString())).toEqual(['2026-10-23T05:00:00.000Z', '2026-10-22T05:00:00.000Z']);
    const everyDay = recentDeadlines(at, { expectedBy: '12:00', days: [], timeZone: 'UTC' });
    expect(everyDay.map((x) => x.toISOString())).toEqual(['2026-10-22T12:00:00.000Z', '2026-10-21T12:00:00.000Z']);
  });
});

describe('expected-arrival alerts and acknowledgement', () => {
  it('raises when a backup has not arrived by its deadline, acks and snoozes stop notifications', async () => {
    const org = createOrg({ name: 'Arrivals' });
    const admin = makeUser('arr@x.org', 'org_admin', org.id);
    const ch = createChannel(org.id, { name: 'hook', kind: 'webhook', config: { url } });
    const { token } = createLink(org.id, {}, 't');
    const { machine } = registerWithToken(token, { name: 'nas', hostname: 'nas' });
    db().prepare('UPDATE machines SET registered_at = ? WHERE id = ?').run('2026-10-01T00:00:00Z', machine.id);
    const m = getMachine(machine.id)!;
    const report = (finishedAt: string, status = 'success', backupId = 'nightly') => ingestReport(org.id, m.id, normalizeReport({ backup_id: backupId, name: 'Nightly', result: status, finished_at: finishedAt }), {});

    // Validation through the API.
    const post = async (params: Record<string, unknown>) => (await rules.POST(await req('POST', '/api/alerts/rules', { as: admin, body: { name: 'r', kind: 'expected_arrival', params, channelIds: [ch.id] } }))).status;
    expect(await post({ expectedBy: '7:00' })).toBe(400);
    expect(await post({ expectedBy: '07:00', timeZone: 'Mars/Olympus' })).toBe(400);
    expect(await post({ expectedBy: '07:00', days: [7] })).toBe(400);

    const rule = createRule(org.id, { name: 'Nightly by 07:00', kind: 'expected_arrival', params: { expectedBy: '07:00', days: [1, 2, 3, 4, 5], timeZone: 'Europe/Berlin', graceMinutes: 30, backupIds: ['nightly'], notifyOnResolve: true }, channelIds: [ch.id], cooldownMinutes: 0 });
    report('2026-10-22T04:00:00Z'); // Thursday, before Thursday's deadline
    report('2026-10-22T04:00:00Z', 'success', 'other'); // not watched

    // Friday 05:20 UTC: deadline 05:00 + 30 min grace not over yet.
    expect(await evaluateMissed(Date.parse('2026-10-23T05:20:00Z'))).toHaveLength(0);
    // Friday 05:40 UTC: nothing since Thursday's deadline -> alert.
    const raised = await evaluateMissed(Date.parse('2026-10-23T05:40:00Z'));
    expect(raised).toHaveLength(1);
    expect(raised[0]).toMatchObject({ kind: 'expected_arrival', backupId: 'nightly' });
    expect(raised[0].message).toContain('did not arrive by 07:00 (Europe/Berlin)');
    expect(hits).toBe(1);

    // Cooldown 0: every evaluation would notify again — unless acknowledged.
    await evaluateMissed(Date.parse('2026-10-23T05:50:00Z'));
    expect(hits).toBe(2);
    const id = raised[0].id;
    const ack = await json<{ alert: { acknowledgedBy: string } }>(await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin, body: { action: 'acknowledge' } }), P(id)));
    expect(ack.body.alert.acknowledgedBy).toBe('arr@x.org');
    await evaluateMissed(Date.parse('2026-10-23T06:00:00Z'));
    expect(hits).toBe(2);
    expect(listAlerts({ orgIds: [org.id], open: true })).toHaveLength(1);

    // Snooze: unacknowledged but snoozed -> still quiet; waking up -> notifies.
    await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin, body: { action: 'unacknowledge' } }), P(id));
    expect((await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin, body: { action: 'snooze', minutes: 99999 } }), P(id))).status).toBe(400);
    await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin, body: { action: 'snooze', minutes: 60 } }), P(id));
    expect(getAlert(org.id, id)!.snoozedUntil).not.toBeNull();
    await evaluateMissed(Date.parse('2026-10-23T06:10:00Z'));
    expect(hits).toBe(2);
    await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin, body: { action: 'snooze', minutes: 0 } }), P(id));
    await evaluateMissed(Date.parse('2026-10-23T06:20:00Z'));
    expect(hits).toBe(3);

    // The backup arrives: resolved (with a notification).
    const late = report('2026-10-23T06:30:00Z');
    await evaluateReport(m, late);
    expect(getAlert(org.id, id)!.resolvedAt).not.toBeNull();
    expect(hits).toBe(4);
    expect((await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin, body: { action: 'acknowledge' } }), P(id))).status).toBe(409);

    // Weekend days are not checked. After DST ends (25 Oct) 07:00 Berlin is
    // 06:00 UTC: Monday's window (Fri 05:00, Mon 06:00] holds Friday's late
    // arrival, Tuesday's (Mon 06:00, Tue 06:00] has nothing.
    expect(await evaluateMissed(Date.parse('2026-10-25T12:00:00Z'))).toHaveLength(0);
    expect(await evaluateMissed(Date.parse('2026-10-26T06:40:00Z'))).toHaveLength(0);
    expect(await evaluateMissed(Date.parse('2026-10-27T06:20:00Z'))).toHaveLength(0);
    expect(await evaluateMissed(Date.parse('2026-10-27T06:40:00Z'))).toHaveLength(1);
    void rule;
  });
});
