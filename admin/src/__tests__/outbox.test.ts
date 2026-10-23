/**
 * Notification outbox: messages sent through channels are recorded, retried
 * on failure (email to an unreachable SMTP server, a webhook answering 500),
 * listed with their status, and can be retried or discarded.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import http from 'http';
import net from 'net';
import type { AddressInfo } from 'net';
import { createOrg } from '@/lib/db/repos/orgs';
import { createChannel, updateChannel } from '@/lib/db/repos/channels';
import { BACKOFF_SECONDS, MAX_ATTEMPTS } from '@/lib/db/repos/webhooks';
import { listOutbox, pruneOutbox } from '@/lib/db/repos/outbox';
import { deliverTo, processOutbox, sendInvitationEmail } from '@/lib/mailer';
import { listAudit } from '@/lib/db/repos/audit';
import { json, makeUser, req } from './helpers';
import * as outbox from '@/app/api/alerts/outbox/route';
import * as outboxById from '@/app/api/alerts/outbox/[id]/route';

const P = (id: string) => ({ params: Promise.resolve({ id }) });
let status = 500;
let hits = 0;
let server: http.Server;
let url = '';
let closedPort = 0;

beforeAll(async () => {
  server = http.createServer((rq, rs) => {
    rq.resume();
    rq.on('end', () => {
      hits++;
      rs.statusCode = status;
      rs.end();
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/hook`;
  // A port with nothing listening: SMTP connections are refused.
  const tmp = net.createServer();
  await new Promise<void>((r) => tmp.listen(0, '127.0.0.1', r));
  closedPort = (tmp.address() as AddressInfo).port;
  await new Promise<void>((r) => tmp.close(() => r()));
});
afterAll(() => server.close());

const later = (s: number) => new Date(Date.now() + s * 1000);

describe('notification outbox', () => {
  it('records sends, retries failures with backoff and gives up', async () => {
    const org = createOrg({ name: 'Outbox org' });
    const admin = makeUser('obx@x.org', 'org_admin', org.id);
    const hook = createChannel(org.id, { name: 'hook', kind: 'webhook', config: { url } });
    const mail = createChannel(org.id, { name: 'mail', kind: 'email', config: { host: '127.0.0.1', port: closedPort, security: 'none', username: '', from: 'ufop@x.org', to: ['ops@x.org'], useForInvitations: true } });

    status = 500;
    const first = await deliverTo(org.id, [hook.id, mail.id, 'chn_missing'], { subject: 'Backup failed', text: 'details', data: { event: 'alert_opened' } });
    expect(first.map((d) => d.ok)).toEqual([false, false, false]);
    expect(first[2].error).toBe('Channel not found');
    const queued = listOutbox(org.id, { status: 'pending' });
    expect(queued).toHaveLength(2);
    expect(queued.every((m) => m.attempts === 1 && m.context === 'alert_opened')).toBe(true);
    expect(listOutbox(org.id).find((m) => m.channelId === mail.id)!.lastError).toMatch(/ECONNREFUSED|connect/i);

    // Not due yet; then the webhook recovers, email keeps failing until it gives up.
    expect(await processOutbox()).toBe(0);
    status = 200;
    expect(await processOutbox(later(BACKOFF_SECONDS[0] + 1))).toBe(2);
    expect(listOutbox(org.id, { status: 'sent' }).map((m) => m.channelId)).toEqual([hook.id]);
    let t = BACKOFF_SECONDS[0] + 1;
    for (let i = 1; i < BACKOFF_SECONDS.length; i++) {
      t += BACKOFF_SECONDS[i] + 1;
      await processOutbox(later(t));
    }
    const dead = listOutbox(org.id, { status: 'dead' });
    expect(dead).toHaveLength(1);
    expect(dead[0].attempts).toBe(MAX_ATTEMPTS);

    // Status page: counts, channel names, no recipients or bodies.
    const page = await json<{ counts: Record<string, number>; channels: Record<string, { name: string }>; messages: { id: string; message: Record<string, unknown> }[] }>(
      await outbox.GET(await req('GET', '/api/alerts/outbox', { as: admin })),
    );
    expect(page.body.counts).toMatchObject({ sent: 1, dead: 1, pending: 0 });
    expect(page.body.channels[mail.id].name).toBe('mail');
    expect(JSON.stringify(page.body)).not.toContain('ops@x.org');
    expect(JSON.stringify(page.body)).not.toContain('details');
    expect((await outbox.GET(await req('GET', '/api/alerts/outbox?status=bogus', { as: admin }))).status).toBe(400);

    // Retry now (still failing) and discard.
    const retry = await json<{ ok: boolean; message: { status: string } }>(await outboxById.POST(await req('POST', `/api/alerts/outbox/${dead[0].id}`, { as: admin, body: { action: 'retry' } }), P(dead[0].id)));
    expect(retry.body.ok).toBe(false);
    expect(retry.body.message.status).toBe('pending');
    expect((await outboxById.POST(await req('POST', `/api/alerts/outbox/${dead[0].id}`, { as: admin, body: { action: 'discard' } }), P(dead[0].id))).status).toBe(200);
    expect(listOutbox(org.id, { status: 'discarded' })).toHaveLength(1);
    const sent = listOutbox(org.id, { status: 'sent' })[0];
    expect((await outboxById.POST(await req('POST', `/api/alerts/outbox/${sent.id}`, { as: admin, body: { action: 'retry' } }), P(sent.id))).status).toBe(409);
    expect((await outboxById.POST(await req('POST', `/api/alerts/outbox/${sent.id}`, { as: admin, body: { action: 'discard' } }), P(sent.id))).status).toBe(409);
    const viewer = makeUser('obx-view@x.org', 'read_only_auditor', org.id);
    expect((await outboxById.POST(await req('POST', `/api/alerts/outbox/${sent.id}`, { as: viewer, body: { action: 'retry' } }), P(sent.id))).status).toBe(403);
    expect(listAudit({ eventType: 'outbox.retry' }).total).toBeGreaterThan(0);

    // Invitations go through the outbox too; disabled channels are not queued.
    const inv = await sendInvitationEmail(org.id, 'new@x.org', 'https://console/invite/x', 'Admin');
    expect(inv.sent).toBe(false);
    expect(listOutbox(org.id).some((m) => m.context === 'invitation')).toBe(true);
    updateChannel(org.id, hook.id, { enabled: false });
    const before = listOutbox(org.id).length;
    expect((await deliverTo(org.id, [hook.id], { subject: 's', text: 't' }))[0].error).toBe('Channel is disabled');
    expect(listOutbox(org.id)).toHaveLength(before);

    // Old sent/discarded messages are pruned; others in other orgs stay invisible.
    expect(pruneOutbox(0, later(1))).toBeGreaterThanOrEqual(2);
    const other = makeUser('obx-other@x.org', 'org_admin');
    const theirs = await json<{ messages: unknown[] }>(await outbox.GET(await req('GET', '/api/alerts/outbox', { as: other })));
    expect(JSON.stringify(theirs.body)).not.toContain(org.id);
    expect(hits).toBeGreaterThan(0);
  });
});
