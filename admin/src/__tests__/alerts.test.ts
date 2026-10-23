/**
 * Alert evaluation and delivery against real transports: a local SMTP server
 * (STARTTLS with a self-signed certificate, AUTH LOGIN; started outside the
 * test, see SMTP_DIR), an HTTPS Slack-style endpoint and a signed JSON webhook.
 */
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import crypto from 'crypto';
import fs from 'fs';
import http from 'http';
import https from 'https';
import net from 'net';
import path from 'path';
import tls from 'tls';
import type { AddressInfo } from 'net';
import { db } from '@/lib/db';
import { createLink, getMachine, registerWithToken } from '@/lib/db/repos/machines';
import { getAlert, listAlerts } from '@/lib/db/repos/alerts';
import { evaluateMissed } from '@/lib/alerts';
import { listAudit, verifyAudit } from '@/lib/db/repos/audit';
import type { UserRecord } from '@/lib/db/repos/users';
import { json, makeUser, req } from './helpers';
import * as channels from '@/app/api/alerts/channels/route';
import * as channelById from '@/app/api/alerts/channels/[id]/route';
import * as channelTest from '@/app/api/alerts/channels/[id]/test/route';
import * as rules from '@/app/api/alerts/rules/route';
import * as ruleById from '@/app/api/alerts/rules/[id]/route';
import * as alertsRoute from '@/app/api/alerts/route';
import * as alertById from '@/app/api/alerts/[id]/route';
import * as agentReport from '@/app/api/agent/report/route';
import * as heartbeat from '@/app/api/agent/heartbeat/route';
import * as users from '@/app/api/users/route';

const P = (id: string) => ({ params: Promise.resolve({ id }) });
const SMTP_DIR = process.env.UFOP_TEST_SMTP_DIR ?? '/tmp/claude-0/emu/mail-admin';
const SMTP_CERT = process.env.UFOP_TEST_SMTP_CERT ?? '/tmp/claude-0/emu/smtp.crt';
const SMTP_KEY = path.join(path.dirname(SMTP_CERT), 'smtp.key');
const SMTP_PORT = 2525;

let smtpUp = false;
const caPem = fs.existsSync(SMTP_CERT) ? fs.readFileSync(SMTP_CERT, 'utf8') : '';

// ── Local HTTP receivers ──

interface Hit { path: string; headers: http.IncomingHttpHeaders; body: string }
let hits: Hit[] = [];
let webhookStatus = 200;
let webhookServer: http.Server;
let slackServer: https.Server;
let webhookUrl = '';
let slackUrl = '';
const defaultCAs = tls.getCACertificates('default');

function collect(rq: http.IncomingMessage, rs: http.ServerResponse) {
  let body = '';
  rq.on('data', (c) => (body += c));
  rq.on('end', () => {
    hits.push({ path: rq.url ?? '', headers: rq.headers, body });
    rs.statusCode = rq.url?.startsWith('/services') ? 200 : webhookStatus;
    rs.end(webhookStatus === 200 ? 'ok' : 'nope');
  });
}

beforeAll(async () => {
  smtpUp = await new Promise<boolean>((r) => {
    const s = net.connect(SMTP_PORT, '127.0.0.1', () => (s.end(), r(true)));
    s.on('error', () => r(false));
  });
  webhookServer = http.createServer(collect);
  await new Promise<void>((r) => webhookServer.listen(0, '127.0.0.1', r));
  webhookUrl = `http://127.0.0.1:${(webhookServer.address() as AddressInfo).port}/hook`;
  // Slack webhooks must be https: serve one with the same self-signed certificate and trust it for this process.
  slackServer = https.createServer({ cert: caPem, key: fs.readFileSync(SMTP_KEY, 'utf8') }, collect);
  await new Promise<void>((r) => slackServer.listen(0, '127.0.0.1', r));
  slackUrl = `https://localhost:${(slackServer.address() as AddressInfo).port}/services/T0/B0/x`;
  tls.setDefaultCACertificates([...defaultCAs, caPem]);
});

afterAll(async () => {
  tls.setDefaultCACertificates(defaultCAs);
  await new Promise<void>((r) => webhookServer.close(() => r()));
  await new Promise<void>((r) => slackServer.close(() => r()));
});

afterEach(() => {
  hits = [];
  webhookStatus = 200;
});

const mails = () => (fs.existsSync(SMTP_DIR) ? fs.readdirSync(SMTP_DIR).filter((f) => f.endsWith('.eml')) : []);
function newMails(before: string[]): string[] {
  return mails().filter((f) => !before.includes(f)).map((f) => fs.readFileSync(path.join(SMTP_DIR, f), 'utf8'));
}
/** Decoded subject + body of a raw message (nodemailer may quote-printable/encode-word them). */
function plain(eml: string): string {
  return eml
    .replace(/=\?UTF-8\?Q\?([^?]*)\?=/gi, (_m, q: string) => Buffer.from(q.replace(/_/g, ' ').replace(/=([0-9A-F]{2})/gi, (_x, h: string) => String.fromCharCode(parseInt(h, 16))), 'latin1').toString('utf8'))
    .replace(/=\r?\n/g, '')
    .replace(/=([0-9A-F]{2})/g, (_x, h: string) => String.fromCharCode(parseInt(h, 16)));
}

const emailConfig = (extra: Record<string, unknown> = {}) => ({ host: 'localhost', port: SMTP_PORT, security: 'starttls', username: 'mailer', from: 'ufop@console.example', to: ['ops@corp.example'], caPem, ...extra });

async function setup(as: UserRecord, withEmail = true) {
  const mk = async (body: unknown) => {
    const r = await json<{ channel: { id: string } }>(await channels.POST(await req('POST', '/api/alerts/channels', { as, body })));
    expect(r.status).toBe(201);
    return r.body.channel.id;
  };
  const ids = {
    webhook: await mk({ name: 'Hook', kind: 'webhook', config: { url: webhookUrl, headers: { 'x-tenant': 'acme' } }, secret: 'hook-signing-secret' }),
    slack: await mk({ name: 'Slack', kind: 'slack', config: {}, secret: slackUrl }),
    email: withEmail ? await mk({ name: 'Mail', kind: 'email', config: emailConfig(), secret: 'm41l' }) : '',
  };
  return ids;
}

function machine(tags: string[] = []) {
  const { token } = createLink('org_default', {}, 't');
  const { machine: m, token: agent } = registerWithToken(token, { name: 'fileserver' }, tags);
  return { id: m.id, token: agent };
}

let clock = Date.parse('2026-10-01T00:00:00Z');
const send = async (token: string, body: Record<string, unknown>) => {
  clock += 3600_000;
  const r = await agentReport.POST(await req('POST', '/api/agent/report', { body: { backup_id: 'docs', name: 'Documents', finished_at: new Date(clock).toISOString(), ...body }, headers: { authorization: `Bearer ${token}` } }));
  expect(r.status).toBe(201);
};
const rule = async (as: UserRecord, body: Record<string, unknown>) => {
  const r = await json<{ rule: { id: string } }>(await rules.POST(await req('POST', '/api/alerts/rules', { as, body })));
  expect(r.status).toBe(201);
  return r.body.rule.id;
};
const open = () => listAlerts({ orgIds: ['org_default'], open: true });

describe('channels', () => {
  it('stores secrets sealed and never returns them; validates input', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const ids = await setup(admin, false);
    const list = await channels.GET(await req('GET', '/api/alerts/channels', { as: admin }));
    const text = await list.text();
    expect(text).not.toContain('hook-signing-secret');
    expect(text).not.toContain(slackUrl);
    expect(JSON.parse(text).channels.every((c: { hasSecret: boolean }) => c.hasSecret)).toBe(true);
    const raw = db().prepare('SELECT secret_ciphertext s FROM notification_channels WHERE id = ?').get(ids.slack) as { s: string };
    expect(raw.s).toMatch(/^v1\./);
    const bad = async (body: unknown) => (await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body }))).status;
    expect(await bad({ name: 'x', kind: 'webhook', config: { url: 'ftp://h' } })).toBe(400);
    expect(await bad({ name: 'x', kind: 'slack', config: {}, secret: 'http://insecure' })).toBe(400);
    expect(await bad({ name: 'x', kind: 'email', config: { host: 'h', from: 'nope', to: [], security: 'starttls' } })).toBe(400);
    expect(await bad({ name: 'x', kind: 'pager', config: {} })).toBe(400);
    // Updating without a secret keeps it; disabling stops delivery.
    expect((await channelById.PUT(await req('PUT', `/api/alerts/channels/${ids.slack}`, { as: admin, body: { name: 'Slack 2', enabled: false } }), P(ids.slack))).status).toBe(200);
    const t = await json<{ ok: boolean; error: string }>(await channelTest.POST(await req('POST', `/api/alerts/channels/${ids.slack}/test`, { as: admin }), P(ids.slack)));
    expect(t).toMatchObject({ status: 502, body: { ok: false, error: 'Channel is disabled' } });
    expect((await channelById.DELETE(await req('DELETE', `/api/alerts/channels/${ids.slack}`, { as: admin }), P(ids.slack))).status).toBe(200);
    expect((await channelTest.POST(await req('POST', `/api/alerts/channels/${ids.slack}/test`, { as: admin }), P(ids.slack))).status).toBe(404);
    // Operators can read channels but not change them.
    expect((await channels.POST(await req('POST', '/api/alerts/channels', { as: makeUser('op@x.org', 'operator'), body: {} }))).status).toBe(403);
    expect(listAudit({}).entries.map((e) => e.eventType)).toEqual(expect.arrayContaining(['alert.channel_create', 'alert.channel_update', 'alert.channel_delete']));
    expect(JSON.stringify(listAudit({}).entries)).not.toContain('hook-signing-secret');
  });

  it('sends signed webhook and Slack test messages', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const ids = await setup(admin, false);
    expect((await channelTest.POST(await req('POST', `/api/alerts/channels/${ids.webhook}/test`, { as: admin }), P(ids.webhook))).status).toBe(200);
    expect((await channelTest.POST(await req('POST', `/api/alerts/channels/${ids.slack}/test`, { as: admin }), P(ids.slack))).status).toBe(200);
    const hook = hits.find((h) => h.path === '/hook')!;
    const expected = `sha256=${crypto.createHmac('sha256', 'hook-signing-secret').update(hook.body).digest('hex')}`;
    expect(hook.headers['x-ufop-signature']).toBe(expected);
    expect(hook.headers['x-tenant']).toBe('acme');
    expect(hook.headers['content-type']).toBe('application/json');
    expect(JSON.parse(hook.body)).toMatchObject({ subject: '[UFOP] Test notification', event: 'test' });
    const slack = hits.find((h) => h.path.startsWith('/services'))!;
    expect(JSON.parse(slack.body)).toEqual({ text: `*[UFOP] Test notification*\nThis is a test from the UFOP backup console, sent by admin@x.org.` });
    // Failures are reported and remembered on the channel.
    webhookStatus = 500;
    const fail = await json<{ ok: boolean; error: string }>(await channelTest.POST(await req('POST', `/api/alerts/channels/${ids.webhook}/test`, { as: admin }), P(ids.webhook)));
    expect(fail).toMatchObject({ status: 502, body: { ok: false, error: 'HTTP 500: nope' } });
    const ch = (await json<{ channels: { id: string; lastError: string | null }[] }>(await channels.GET(await req('GET', '/api/alerts/channels', { as: admin })))).body.channels.find((c) => c.id === ids.webhook);
    expect(ch?.lastError).toBe('HTTP 500: nope');
  });

  it('posts an Adaptive Card to a Microsoft Teams workflow', async () => {
    const admin = makeUser('teams@x.org', 'org_admin');
    const bad = async (secret: string | undefined) => (await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body: { name: 'T', kind: 'teams', config: {}, secret } }))).status;
    expect(await bad(undefined)).toBe(400);
    expect(await bad('http://insecure.example/x')).toBe(400);
    const teamsUrl = slackUrl.replace('/services/T0/B0/x', '/services/teams/workflow');
    const id = (await json<{ channel: { id: string } }>(await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body: { name: 'Teams', kind: 'teams', config: {}, secret: teamsUrl } })))).body.channel.id;
    const listed = await (await channels.GET(await req('GET', '/api/alerts/channels', { as: admin }))).text();
    expect(listed).not.toContain(teamsUrl);
    expect((await channelTest.POST(await req('POST', `/api/alerts/channels/${id}/test`, { as: admin }), P(id))).status).toBe(200);
    const hit = hits.find((h) => h.path === '/services/teams/workflow')!;
    const card = JSON.parse(hit.body);
    expect(card.type).toBe('message');
    expect(card.attachments[0].contentType).toBe('application/vnd.microsoft.card.adaptive');
    const body = card.attachments[0].content.body;
    expect(body[0]).toMatchObject({ type: 'TextBlock', text: '[UFOP] Test notification', weight: 'Bolder' });
    expect(body[1].text).toContain('sent by teams@x.org');
    expect(body[2]).toEqual({ type: 'FactSet', facts: [{ title: 'event', value: 'test' }] });
  });

  it('delivers email over STARTTLS with AUTH, trusting only the configured CA', async () => {
    if (!smtpUp) throw new Error(`Start the local SMTP server on 127.0.0.1:${SMTP_PORT} (see the file header)`);
    const admin = makeUser('admin@x.org', 'org_admin');
    const before = mails();
    // Without the CA in the default trust store, only the channel's caPem makes the server trusted.
    tls.setDefaultCACertificates(defaultCAs);
    try {
      const ok = await json<{ channel: { id: string } }>(await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body: { name: 'Mail', kind: 'email', config: emailConfig(), secret: 'm41l' } })));
      const t = await json<{ ok: boolean; error?: string }>(await channelTest.POST(await req('POST', `/api/alerts/channels/${ok.body.channel.id}/test`, { as: admin }), P(ok.body.channel.id)));
      expect(t.body).toEqual({ channelId: ok.body.channel.id, ok: true });
      const got = newMails(before);
      expect(got).toHaveLength(1);
      expect(got[0]).toContain('X-Rcpt: ops@corp.example');
      expect(plain(got[0])).toContain('Subject: [UFOP] Test notification');
      expect(plain(got[0])).toContain('sent by admin@x.org');

      const untrusted = await json<{ channel: { id: string } }>(await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body: { name: 'Mail no CA', kind: 'email', config: emailConfig({ caPem: null }), secret: 'm41l' } })));
      const t2 = await json<{ ok: boolean; error: string }>(await channelTest.POST(await req('POST', `/api/alerts/channels/${untrusted.body.channel.id}/test`, { as: admin }), P(untrusted.body.channel.id)));
      expect(t2.status).toBe(502);
      expect(t2.body.error).toMatch(/certificate/i);

      const badPw = await json<{ channel: { id: string } }>(await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body: { name: 'Mail bad pw', kind: 'email', config: emailConfig(), secret: 'wrong' } })));
      const t3 = await json<{ error: string }>(await channelTest.POST(await req('POST', `/api/alerts/channels/${badPw.body.channel.id}/test`, { as: admin }), P(badPw.body.channel.id)));
      expect(t3.status).toBe(502);
      expect(t3.body.error).toMatch(/auth|535|credentials/i);
      expect(newMails(before)).toHaveLength(1);
    } finally {
      tls.setDefaultCACertificates([...defaultCAs, caPem]);
    }
  });

  it('emails invitations through the channel marked for invitations', async () => {
    if (!smtpUp) throw new Error('SMTP server not running');
    const admin = makeUser('admin@x.org', 'org_admin');
    await channels.POST(await req('POST', '/api/alerts/channels', { as: admin, body: { name: 'Mail', kind: 'email', config: emailConfig({ useForInvitations: true }), secret: 'm41l' } }));
    const before = mails();
    const r = await json<{ emailed: boolean; link: string }>(await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'newbie@corp.example', role: 'operator' } })));
    expect(r.body.emailed).toBe(true);
    const [m] = newMails(before);
    expect(m).toContain('X-Rcpt: newbie@corp.example');
    expect(plain(m)).toContain(r.body.link);
  });
});

describe('alert rules', () => {
  it('raises backup_failed, notifies every channel once per cooldown and resolves on success', async () => {
    if (!smtpUp) throw new Error('SMTP server not running');
    const admin = makeUser('admin@x.org', 'org_admin');
    const ids = await setup(admin);
    const m = machine();
    const rid = await rule(admin, { name: 'Failures', kind: 'backup_failed', channelIds: [ids.webhook, ids.slack, ids.email], params: { notifyOnResolve: true } });
    expect((await rules.POST(await req('POST', '/api/alerts/rules', { as: admin, body: { name: 'x', kind: 'backup_failed', channelIds: ['chn_missing'] } }))).status).toBe(400);
    const before = mails();

    await send(m.token, { result: 'error', message: 'Destination unreachable', data: { errors: ['e1'] } });
    const [a] = open();
    expect(a).toMatchObject({ kind: 'backup_failed', severity: 'critical', machineId: m.id, backupId: 'docs' });
    expect(a.notified.map((n) => n.ok)).toEqual([true, true, true]);
    const hook = hits.find((h) => h.path === '/hook')!;
    expect(hook.headers['x-ufop-signature']).toBe(`sha256=${crypto.createHmac('sha256', 'hook-signing-secret').update(hook.body).digest('hex')}`);
    expect(JSON.parse(hook.body)).toMatchObject({ event: 'alert', alertId: a.id, kind: 'backup_failed', severity: 'critical', machine: { id: m.id, name: 'fileserver' }, backupId: 'docs', rule: { id: rid, name: 'Failures' }, details: { errors: 1, message: 'Destination unreachable' } });
    expect(JSON.parse(hits.find((h) => h.path.startsWith('/services'))!.body).text).toMatch(/^\*\[UFOP\] Critical: Backup "Documents" on fileserver failed\*/);
    const mail = newMails(before);
    expect(mail).toHaveLength(1);
    // Long subjects are folded across header lines.
    expect(plain(mail[0]).replace(/\s+/g, '')).toContain('Subject:[UFOP]Critical:Backup"Documents"onfileserverfailed');
    expect(plain(mail[0])).toContain('message: Destination unreachable');

    // Still failing: same alert, no new notification within the cooldown.
    hits = [];
    await send(m.token, { result: 'error' });
    expect(open()).toHaveLength(1);
    expect(hits).toHaveLength(0);
    // Cooldown 0: the next failure re-notifies on the same alert.
    expect((await ruleById.PUT(await req('PUT', `/api/alerts/rules/${rid}`, { as: admin, body: { cooldownMinutes: 0 } }), P(rid))).status).toBe(200);
    await send(m.token, { result: 'error' });
    expect(hits.filter((h) => h.path === '/hook')).toHaveLength(1);
    expect(getAlert('org_default', a.id)!.notified).toHaveLength(6);

    // Warnings do not count unless the rule says so.
    hits = [];
    await send(m.token, { result: 'warning' });
    expect(open()).toHaveLength(0);
    const resolved = getAlert('org_default', a.id)!;
    expect(resolved.resolvedAt).toBeTruthy();
    const res = JSON.parse(hits.find((h) => h.path === '/hook')!.body);
    expect(res).toMatchObject({ event: 'alert_resolved', alertId: a.id, subject: `[UFOP] Resolved: ${a.message}` });
    await ruleById.PUT(await req('PUT', `/api/alerts/rules/${rid}`, { as: admin, body: { params: { includeWarnings: true } } }), P(rid));
    await send(m.token, { result: 'warning' });
    expect(open()).toMatchObject([{ severity: 'warning' }]);
    expect(listAudit({ eventType: 'alert.raise' }).total).toBe(2);
  });

  it('flags unusual change volumes', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = machine();
    expect((await rules.POST(await req('POST', '/api/alerts/rules', { as: admin, body: { name: 'x', kind: 'changed_files' } }))).status).toBe(400);
    expect((await rules.POST(await req('POST', '/api/alerts/rules', { as: admin, body: { name: 'x', kind: 'changed_files', params: { maxChangedFiles: -1 } } }))).status).toBe(400);
    await rule(admin, { name: 'Ransomware', kind: 'changed_files', params: { maxChangedFiles: 50, maxChangedPercent: 30, maxDeletedFiles: 5 } });
    const files = (added: number, modified: number, deleted: number) => ({ result: 'success', data: { files_examined: 1000, files_added: added, files_changed: modified, files_deleted: deleted } });
    await send(m.token, files(10, 10, 2));
    expect(open()).toHaveLength(0);
    await send(m.token, files(0, 40, 20));
    const [a] = open();
    expect(a.kind).toBe('changed_files');
    expect(a.message).toContain('60 files changed (limit 50)');
    expect(a.message).toContain('20 files deleted (limit 5)');
    expect(a.message).not.toContain('% of files');
    // Failed runs are not judged by volume.
    await send(m.token, { result: 'error', data: { files_examined: 1000, files_added: 900 } });
    expect(open()).toHaveLength(1);
    await send(m.token, files(1, 1, 0));
    expect(open()).toHaveLength(0);
    await send(m.token, files(400, 0, 0));
    expect(open()[0].message).toContain('40.0% of files changed (limit 30%)');
  });

  it('detects duration anomalies against the baseline', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = machine();
    await rule(admin, { name: 'Duration', kind: 'duration_anomaly', params: { minSamples: 5, factor: 2, stdDevs: 3, detectShort: true } });
    const run = (s: number) => send(m.token, { result: 'success', duration: `${s}s` });
    // Not enough samples yet: no judgement.
    await run(100);
    await run(900);
    expect(open()).toHaveLength(0);
    db().prepare('DELETE FROM backup_reports').run();
    for (const s of [100, 110, 95, 105, 100]) await run(s);
    expect(open()).toHaveLength(0);
    await run(150);
    expect(open()).toHaveLength(0);
    await run(400);
    const [a] = open();
    expect(a).toMatchObject({ kind: 'duration_anomaly', details: { direction: 'longer', medianMs: 102500, samples: 6 } });
    expect(a.message).toContain('took 400s, usually 103s');
    await run(100);
    expect(open()).toHaveLength(0);
    await run(10);
    expect(open()[0].details).toMatchObject({ direction: 'shorter' });
  });

  it('raises missed backups from the timer and resolves them when the backup runs', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const ids = await setup(admin, false);
    const m = machine(['servers']);
    const other = machine();
    await rule(admin, { name: 'Missed', kind: 'missed_backup', params: { graceMinutes: 30, notifyOnResolve: true }, scope: { tags: ['Servers'] }, channelIds: [ids.webhook] });
    // The schedule comes from the heartbeat; the last success from the report.
    await heartbeat.POST(await req('POST', '/api/agent/heartbeat', { body: { jobs: [{ id: 'docs', name: 'Documents', scheduleIntervalMinutes: 60 }] }, headers: { authorization: `Bearer ${m.token}` } }));
    await send(m.token, { result: 'success' });
    await send(other.token, { result: 'success', stats: { schedule_interval_minutes: 60 } });
    const last = clock - 3600_000; // m's report
    expect(await evaluateMissed(last + 80 * 60_000)).toHaveLength(0);
    const raised = await evaluateMissed(last + 100 * 60_000);
    expect(raised).toHaveLength(1);
    expect(raised[0]).toMatchObject({ kind: 'missed_backup', severity: 'critical', machineId: m.id, details: { expectedEveryMinutes: 60, overdueMinutes: 10 } });
    expect(JSON.parse(hits[0].body)).toMatchObject({ event: 'alert', kind: 'missed_backup' });
    // Repeated timer runs keep one open alert.
    await evaluateMissed(last + 200 * 60_000);
    expect(open()).toHaveLength(1);
    // Suspended machines are not chased.
    db().prepare("UPDATE machines SET status = 'suspended' WHERE id = ?").run(m.id);
    db().prepare('UPDATE alerts SET resolved_at = ?').run(new Date().toISOString());
    expect(await evaluateMissed(last + 300 * 60_000)).toHaveLength(0);
    db().prepare("UPDATE machines SET status = 'active' WHERE id = ?").run(m.id);
    await evaluateMissed(last + 300 * 60_000);
    hits = [];
    await send(m.token, { result: 'success' });
    expect(open()).toHaveLength(0);
    expect(JSON.parse(hits[0].body)).toMatchObject({ event: 'alert_resolved' });
  });

  it('assumes a default interval for backups without a schedule', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = machine();
    await send(m.token, { result: 'success' });
    const last = clock;
    await rule(admin, { name: 'Missed', kind: 'missed_backup', params: { graceMinutes: 0 } });
    expect(await evaluateMissed(last + 10 * 86_400_000)).toHaveLength(0);
    const rid = (await json<{ rules: { id: string }[] }>(await rules.GET(await req('GET', '/api/alerts/rules', { as: admin })))).body.rules[0].id;
    await ruleById.PUT(await req('PUT', `/api/alerts/rules/${rid}`, { as: admin, body: { params: { graceMinutes: 0, defaultIntervalHours: 24 } } }), P(rid));
    expect(await evaluateMissed(last + 23 * 3600_000)).toHaveLength(0);
    expect(await evaluateMissed(last + 25 * 3600_000)).toHaveLength(1);
    // Disabled rules do nothing.
    db().prepare('UPDATE alerts SET resolved_at = ?').run(new Date().toISOString());
    await ruleById.PUT(await req('PUT', `/api/alerts/rules/${rid}`, { as: admin, body: { enabled: false } }), P(rid));
    expect(await evaluateMissed(last + 50 * 3600_000)).toHaveLength(0);
    expect((await ruleById.DELETE(await req('DELETE', `/api/alerts/rules/${rid}`, { as: admin }), P(rid))).status).toBe(200);
    expect((await ruleById.DELETE(await req('DELETE', `/api/alerts/rules/${rid}`, { as: admin }), P(rid))).status).toBe(404);
    expect(listAudit({}).entries.map((e) => e.eventType)).toEqual(expect.arrayContaining(['alert.rule_create', 'alert.rule_update', 'alert.rule_delete']));
  });

  it('lists and resolves alerts by hand, scoped to rules and machines', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = machine(['prod']);
    const dev = machine(['dev']);
    await rule(admin, { name: 'Prod failures', kind: 'backup_failed', scope: { tags: ['prod'] } });
    await send(m.token, { result: 'error' });
    await send(dev.token, { result: 'error' });
    const list = await json<{ alerts: { id: string; machineId: string }[] }>(await alertsRoute.GET(await req('GET', '/api/alerts', { as: admin })));
    expect(list.body.alerts.map((a) => a.machineId)).toEqual([m.id]);
    const id = list.body.alerts[0].id;
    expect((await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: makeUser('aud@x.org', 'read_only_auditor') }), P(id))).status).toBe(403);
    const r = await json<{ alert: { resolvedAt: string } }>(await alertById.PATCH(await req('PATCH', `/api/alerts/${id}`, { as: admin }), P(id)));
    expect(r.body.alert.resolvedAt).toBeTruthy();
    expect((await json<{ alerts: unknown[] }>(await alertsRoute.GET(await req('GET', '/api/alerts', { as: admin })))).body.alerts).toHaveLength(0);
    expect((await json<{ alerts: unknown[] }>(await alertsRoute.GET(await req('GET', '/api/alerts?state=resolved', { as: admin })))).body.alerts).toHaveLength(1);
    expect((await alertById.PATCH(await req('PATCH', '/api/alerts/alt_nope', { as: admin }), P('alt_nope'))).status).toBe(404);
    expect(getMachine(dev.id)!.tags).toEqual(['dev']);
    const types = listAudit({}).entries.map((e) => e.eventType);
    expect(types).toEqual(expect.arrayContaining(['alert.rule_create', 'alert.raise', 'alert.resolve']));
    expect(verifyAudit()).toMatchObject({ ok: true });
  });
});
