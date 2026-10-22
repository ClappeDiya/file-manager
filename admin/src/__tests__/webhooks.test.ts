/**
 * Outbound webhooks: subscriptions, signing, the persisted delivery queue
 * with retries and auto-pause, redelivery, the internal-address guard, and
 * events from audit, alerts and backup reports.
 */
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import http from 'http';
import crypto from 'crypto';
import type { AddressInfo } from 'net';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { appendAudit, listAudit } from '@/lib/db/repos/audit';
import { insertAlert, resolveAlert } from '@/lib/db/repos/alerts';
import { BACKOFF_SECONDS, enqueueEvent, eventMatches, MAX_ATTEMPTS, PAUSE_AFTER_FAILURES, pruneDeliveries } from '@/lib/db/repos/webhooks';
import { isPrivateAddress, processDueDeliveries, send, sign } from '@/lib/delivery';
import { json, makeUser, req } from './helpers';
import * as webhooks from '@/app/api/webhooks/route';
import * as webhookById from '@/app/api/webhooks/[id]/route';

interface Received {
  headers: http.IncomingHttpHeaders;
  body: string;
}
const received: Received[] = [];
let status = 200;
let server: http.Server;
let base = '';

beforeAll(async () => {
  process.env.WEBHOOKS_ALLOW_PRIVATE = '1';
  server = http.createServer((rq, rs) => {
    let body = '';
    rq.on('data', (c) => (body += c));
    rq.on('end', () => {
      received.push({ headers: rq.headers, body });
      if (rq.url === '/redirect') {
        rs.writeHead(302, { location: 'http://169.254.169.254/' });
        return rs.end();
      }
      rs.writeHead(status);
      rs.end('ok');
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
afterAll(() => {
  server.close();
  delete process.env.WEBHOOKS_ALLOW_PRIVATE;
});
afterEach(() => {
  received.length = 0;
  status = 200;
});

const P = (id: string) => ({ params: Promise.resolve({ id }) });
const later = (s: number) => new Date(Date.now() + s * 1000);

async function create(as: ReturnType<typeof makeUser>, body: Record<string, unknown>) {
  return json<{ webhook: { id: string; name: string; events: string[] }; secret: string }>(await webhooks.POST(await req('POST', '/api/webhooks', { as, body })));
}

describe('webhooks', () => {
  it('matches events exactly, by prefix and by wildcard', () => {
    expect(eventMatches(['backup.*'], 'backup.error')).toBe(true);
    expect(eventMatches(['backup.*'], 'backups.error')).toBe(false);
    expect(eventMatches(['machine.register'], 'machine.register')).toBe(true);
    expect(eventMatches(['machine.register'], 'machine.revoke')).toBe(false);
    expect(eventMatches(['*'], 'anything.at.all')).toBe(true);
  });

  it('validates subscriptions and keeps the secret write-only', async () => {
    const admin = makeUser('wh-admin@x.org', 'org_admin');
    const viewer = makeUser('wh-view@x.org', 'read_only_auditor');
    expect((await create(viewer, { name: 'n', url: `${base}/h`, events: ['*'] })).status).toBe(403);
    for (const bad of [
      { name: '', url: `${base}/h`, events: ['*'] },
      { name: 'n', url: 'ftp://x/y', events: ['*'] },
      { name: 'n', url: 'https://user:pw@x.example/y', events: ['*'] },
      { name: 'n', url: `${base}/h`, events: [] },
      { name: 'n', url: `${base}/h`, events: ['Bad Event'] },
      { name: 'n', url: `${base}/h`, events: ['*'], headers: { 'x-ufop-signature': 'forged' } },
      { name: 'n', url: `${base}/h`, events: ['*'], headers: { 'x-a': 'line\r\nbreak' } },
    ]) expect((await create(admin, bad)).status).toBe(400);
    const ok = await create(admin, { name: 'SIEM', url: `${base}/h`, events: ['backup.*', 'alert.opened'], headers: { Authorization: 'Bearer abc' } });
    expect(ok.status).toBe(201);
    expect(ok.body.secret).toMatch(/^whsec_/);
    const list = await webhooks.GET(await req('GET', '/api/webhooks', { as: viewer }));
    const text = await list.text();
    expect(text).not.toContain(ok.body.secret);
    expect(JSON.parse(text).events.length).toBeGreaterThan(3);
    const stored = db().prepare('SELECT secret_ciphertext FROM webhooks WHERE id = ?').get(ok.body.webhook.id) as { secret_ciphertext: string };
    expect(stored.secret_ciphertext).not.toContain(ok.body.secret);
  });

  it('delivers signed events from audit, alerts and backup reports', async () => {
    const admin = makeUser('wh-ev@x.org', 'org_admin');
    const org = createOrg({ name: 'Webhook events' });
    const hookAdmin = makeUser('wh-ev2@x.org', 'org_admin', org.id);
    const { body } = await create(hookAdmin, { name: 'all', url: `${base}/h`, events: ['machine.*', 'alert.*', 'backup.error'], headers: { authorization: 'Bearer abc' } });
    void admin;

    appendAudit({ eventType: 'machine.register', orgId: org.id, description: 'Machine "x" registered', resourceType: 'machine', resourceId: 'mch_x' });
    appendAudit({ eventType: 'credential.create', orgId: org.id, description: 'not subscribed' });
    appendAudit({ eventType: 'machine.register', orgId: 'org_default', description: 'other organization' });
    const alert = insertAlert({ orgId: org.id, ruleId: null, machineId: 'mch_x', backupId: 'b1', kind: 'backup_failed', severity: 'critical', message: 'Backup failed', details: {}, fingerprint: 'fp-wh-1' });
    resolveAlert(alert.id);
    resolveAlert(alert.id); // only once
    expect(enqueueEvent(org.id, 'backup.success', {})).toBe(0);
    expect(enqueueEvent(org.id, 'backup.error', { backupId: 'b1' })).toBe(1);

    expect(await processDueDeliveries()).toBe(4);
    expect(received.map((r) => r.headers['x-ufop-event'])).toEqual(['machine.register', 'alert.opened', 'alert.resolved', 'backup.error']);
    const r = received[0];
    expect(r.headers.authorization).toBe('Bearer abc');
    expect(r.headers['content-type']).toBe('application/json');
    const ts = Number(r.headers['x-ufop-timestamp']);
    expect(Math.abs(ts - Date.now() / 1000)).toBeLessThan(60);
    const expected = `sha256=${crypto.createHmac('sha256', body.secret).update(`${ts}.${r.body}`).digest('hex')}`;
    expect(r.headers['x-ufop-signature']).toBe(expected);
    const payload = JSON.parse(r.body);
    expect(payload).toMatchObject({ event: 'machine.register', organizationId: org.id, data: { description: 'Machine "x" registered', resourceId: 'mch_x' } });
    expect(payload.id).toBe(r.headers['x-ufop-delivery']);

    const log = await json<{ deliveries: { status: string; lastStatus: number }[]; stats: Record<string, number> }>(await webhookById.GET(await req('GET', `/api/webhooks/${body.webhook.id}`, { as: hookAdmin }), P(body.webhook.id)));
    expect(log.body.stats).toEqual({ pending: 0, succeeded: 4, dead: 0 });
    expect(log.body.deliveries.every((d) => d.status === 'succeeded' && d.lastStatus === 200)).toBe(true);
  });

  it('retries with backoff, gives up, pauses a failing webhook and redelivers on request', async () => {
    const org = createOrg({ name: 'Webhook retries' });
    const admin = makeUser('wh-rt@x.org', 'org_admin', org.id);
    const { body } = await create(admin, { name: 'flaky', url: `${base}/h`, events: ['test.*'] });
    const id = body.webhook.id;
    status = 503;
    enqueueEvent(org.id, 'test.one', {});
    expect(await processDueDeliveries()).toBe(1);
    expect(await processDueDeliveries()).toBe(0); // not due yet
    let t = 0;
    for (let i = 0; i < BACKOFF_SECONDS.length; i++) {
      t += BACKOFF_SECONDS[i] + 1;
      expect(await processDueDeliveries(later(t))).toBe(1);
    }
    const dead = (await json<{ deliveries: { id: string; status: string; attempts: number; lastError: string }[] }>(await webhookById.GET(await req('GET', `/api/webhooks/${id}?status=dead`, { as: admin }), P(id)))).body.deliveries;
    expect(dead).toHaveLength(1);
    expect(dead[0]).toMatchObject({ status: 'dead', attempts: MAX_ATTEMPTS });
    expect(dead[0].lastError).toContain('HTTP 503');

    // Redeliver after the receiver recovers.
    status = 200;
    expect((await webhookById.POST(await req('POST', `/api/webhooks/${id}`, { as: admin, body: { action: 'redeliver', deliveryId: dead[0].id } }), P(id))).status).toBe(200);
    expect(await processDueDeliveries()).toBe(1);

    // Many failures in a row pause the webhook; new events are not queued while paused.
    status = 500;
    for (let i = 0; i < PAUSE_AFTER_FAILURES; i++) enqueueEvent(org.id, 'test.burst', { i });
    await processDueDeliveries();
    const paused = (await json<{ webhook: { pausedReason: string | null } }>(await webhookById.GET(await req('GET', `/api/webhooks/${id}`, { as: admin }), P(id)))).body.webhook;
    expect(paused.pausedReason).toContain(`${PAUSE_AFTER_FAILURES} failed deliveries`);
    expect(enqueueEvent(org.id, 'test.burst', {})).toBe(0);
    expect((await webhookById.POST(await req('POST', `/api/webhooks/${id}`, { as: admin, body: { action: 'test' } }), P(id))).status).toBe(409);
    status = 200;
    expect((await webhookById.POST(await req('POST', `/api/webhooks/${id}`, { as: admin, body: { action: 'resume' } }), P(id))).status).toBe(200);
    // Pending retries continue once resumed.
    expect(await processDueDeliveries(later(BACKOFF_SECONDS[0] + 1))).toBeGreaterThan(0);
    expect(received.at(-1)!.headers['x-ufop-event']).toBe('test.burst');

    const test = await json<{ deliveryId: string }>(await webhookById.POST(await req('POST', `/api/webhooks/${id}`, { as: admin, body: { action: 'test' } }), P(id)));
    await processDueDeliveries();
    expect(received.at(-1)!.headers['x-ufop-delivery']).toBe(test.body.deliveryId);
    expect(listAudit({ eventType: 'webhook.test' }).total).toBeGreaterThan(0);
  });

  it('rotates the secret, edits, disables and deletes', async () => {
    const org = createOrg({ name: 'Webhook edits' });
    const admin = makeUser('wh-ed@x.org', 'org_admin', org.id);
    const { body } = await create(admin, { name: 'edit me', url: `${base}/h`, events: ['x.*'] });
    const id = body.webhook.id;
    const rot = await json<{ secret: string }>(await webhookById.POST(await req('POST', `/api/webhooks/${id}`, { as: admin, body: { action: 'rotate_secret' } }), P(id)));
    expect(rot.body.secret).not.toBe(body.secret);
    enqueueEvent(org.id, 'x.y', {});
    await processDueDeliveries();
    const r = received[0];
    expect(r.headers['x-ufop-signature']).toBe(sign(rot.body.secret, Number(r.headers['x-ufop-timestamp']), r.body));

    expect((await webhookById.PATCH(await req('PATCH', `/api/webhooks/${id}`, { as: admin, body: { enabled: false, events: ['y.*'] } }), P(id))).status).toBe(200);
    expect(enqueueEvent(org.id, 'y.z', {})).toBe(0);
    expect((await webhookById.PATCH(await req('PATCH', `/api/webhooks/${id}`, { as: admin, body: { url: 'nope' } }), P(id))).status).toBe(400);
    const other = makeUser('wh-ed-other@x.org', 'org_admin');
    expect((await webhookById.DELETE(await req('DELETE', `/api/webhooks/${id}`, { as: other }), P(id))).status).toBe(404);
    expect((await webhookById.DELETE(await req('DELETE', `/api/webhooks/${id}`, { as: admin }), P(id))).status).toBe(200);
    expect(db().prepare('SELECT COUNT(*) AS n FROM webhook_deliveries WHERE webhook_id = ?').get(id)).toEqual({ n: 0 });
    expect(pruneDeliveries()).toBeGreaterThanOrEqual(0);
  });

  it('refuses internal addresses unless allowed and does not follow redirects', async () => {
    expect(isPrivateAddress('127.0.0.1')).toBe(true);
    expect(isPrivateAddress('169.254.169.254')).toBe(true);
    expect(isPrivateAddress('10.1.2.3')).toBe(true);
    expect(isPrivateAddress('172.20.0.1')).toBe(true);
    expect(isPrivateAddress('192.168.1.1')).toBe(true);
    expect(isPrivateAddress('::1')).toBe(true);
    expect(isPrivateAddress('fd00::1')).toBe(true);
    expect(isPrivateAddress('::ffff:10.0.0.1')).toBe(true);
    expect(isPrivateAddress('93.184.216.34')).toBe(false);
    expect(isPrivateAddress('2606:4700::1')).toBe(false);

    const d = { id: 'dlv_x', event: 'x', payload: {} };
    expect((await send(`${base}/redirect`, {}, 's', d)).error).toContain('redirects are not followed');
    delete process.env.WEBHOOKS_ALLOW_PRIVATE;
    try {
      expect((await send(`${base}/h`, {}, 's', d)).error).toContain('internal address');
      expect((await send('http://localhost:1/h', {}, 's', d)).error).toContain('internal address');
      expect(received).toHaveLength(1); // only the redirect request
    } finally {
      process.env.WEBHOOKS_ALLOW_PRIVATE = '1';
    }
  });
});
