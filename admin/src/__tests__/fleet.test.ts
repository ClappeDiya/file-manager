/**
 * Fleet and agent protocol through the route handlers: enrollment (links,
 * claim codes, report keys), heartbeats, reports and backup rollups, token
 * rotation, revocation, tags, dashboards, remote commands, fleet policies
 * and central credentials.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { NextRequest } from 'next/server';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { getMachine } from '@/lib/db/repos/machines';
import { listBackups } from '@/lib/db/repos/reports';
import { getCommand } from '@/lib/db/repos/commands';
import { listAudit, verifyAudit } from '@/lib/db/repos/audit';
import type { UserRecord } from '@/lib/db/repos/users';
import { json, makeUser, req } from './helpers';
import * as links from '@/app/api/fleet/links/route';
import * as linkById from '@/app/api/fleet/links/[id]/route';
import * as preload from '@/app/api/fleet/links/[id]/preload/route';
import * as claims from '@/app/api/fleet/claims/route';
import * as machines from '@/app/api/fleet/machines/route';
import * as machineById from '@/app/api/fleet/machines/[id]/route';
import * as commands from '@/app/api/fleet/machines/[id]/commands/route';
import * as tags from '@/app/api/fleet/tags/route';
import * as dashboard from '@/app/api/fleet/dashboard/route';
import * as reports from '@/app/api/fleet/reports/route';
import * as fleetPolicies from '@/app/api/fleet/policies/route';
import * as fleetPolicyById from '@/app/api/fleet/policies/[id]/route';
import * as credentials from '@/app/api/fleet/credentials/route';
import * as credentialById from '@/app/api/fleet/credentials/[id]/route';
import * as register from '@/app/api/agent/register/route';
import * as claim from '@/app/api/agent/claim/route';
import * as claimPoll from '@/app/api/agent/claim/poll/route';
import * as heartbeat from '@/app/api/agent/heartbeat/route';
import * as agentReport from '@/app/api/agent/report/route';
import * as agentDiagnostics from '@/app/api/agent/diagnostics/route';
import * as fleetConnect from '@/app/api/fleet/connect/route';
import * as managedSettings from '@/app/api/fleet/managed-settings/route';
import * as connectToken from '@/app/api/agent/connect/token/route';
import { middleware } from '@/middleware';
import { createHash } from 'node:crypto';
import * as machineDiagnostics from '@/app/api/fleet/machines/[id]/diagnostics/route';
import { KEEP_PER_MACHINE, MAX_DIAGNOSTICS_BYTES } from '@/lib/db/repos/diagnostics';
import * as rotate from '@/app/api/agent/rotate/route';
import * as unregister from '@/app/api/agent/unregister/route';
import * as agentCommand from '@/app/api/agent/commands/[id]/route';
import * as agentPolicies from '@/app/api/agent/policies/route';
import * as applied from '@/app/api/agent/policies/applied/route';
import * as ingest from '@/app/api/ingest/[token]/route';

const P = (id: string) => ({ params: Promise.resolve({ id }) });
const bearer = (t: string) => ({ authorization: `Bearer ${t}` });

afterEach(() => {
  delete process.env.ADMIN_PUBLIC_URL;
});

async function newLink(as: UserRecord, body: Record<string, unknown> = {}) {
  const r = await json<{ link: { id: string; defaultTags: string[]; tokenPrefix: string }; token: string; reportUrl: string; consoleUrl: string }>(await links.POST(await req('POST', '/api/fleet/links', { as, body })));
  expect(r.status).toBe(201);
  return r.body;
}

async function enroll(as: UserRecord, name: string, tagList: string[] = []) {
  const { token } = await newLink(as);
  const r = await json<{ machineId: string; agentToken: string }>(await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: token, info: { name, hostname: name, os: 'linux', clientVersion: '1.0.0' }, tags: tagList } })));
  expect(r.status).toBe(201);
  return { id: r.body.machineId, token: r.body.agentToken };
}

const hb = async (token: string, body: unknown = {}) => heartbeat.POST(await req('POST', '/api/agent/heartbeat', { body, headers: bearer(token) }));
const sendReport = async (token: string, body: unknown) => agentReport.POST(await req('POST', '/api/agent/report', { body, headers: bearer(token) }));

function rawReq(url: string, body: string, type: string): NextRequest {
  return new NextRequest(new URL(url, 'http://localhost:3001'), { method: 'POST', headers: { 'content-type': type, 'x-forwarded-for': '203.0.113.9' }, body });
}

describe('enrollment', () => {
  it('registers with a link token and honours use limits, revocation and preload files', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    process.env.ADMIN_PUBLIC_URL = 'https://console.example.com/';
    const l = await newLink(admin, { label: 'HQ', maxUses: 1, defaultTags: ['Office HQ'] });
    expect(l.token).toMatch(/^ufop_reg_/);
    expect(l.reportUrl).toBe(`https://console.example.com/api/ingest/${encodeURIComponent(l.token)}`);
    expect(JSON.stringify((await json(await links.GET(await req('GET', '/api/fleet/links', { as: admin })))).body)).not.toContain(l.token);
    expect((await links.POST(await req('POST', '/api/fleet/links', { as: admin, body: { maxUses: 0 } }))).status).toBe(400);

    const pre = await preload.GET(await req('GET', `/api/fleet/links/${l.link.id}/preload`, { as: admin }), P(l.link.id));
    expect(pre.headers.get('content-disposition')).toContain('preload.json');
    expect(await pre.json()).toEqual({ env: { '*': { UFOP_CONSOLE_URL: 'https://console.example.com', UFOP_CONSOLE_REGISTRATION_TOKEN: l.token, UFOP_CONSOLE_TAGS: 'office-hq' } }, args: {}, db: {} });

    const r = await json<{ machineId: string; agentToken: string; organization: string }>(await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: l.token, info: { name: 'ws-1', os: 'windows', agentKind: 'server' }, tags: ['Finance'] } })));
    expect(r.status).toBe(201);
    expect(r.body.agentToken).toMatch(/^ufop_agent_/);
    expect(r.body.organization).toBe('Default organization');
    expect(getMachine(r.body.machineId)).toMatchObject({ name: 'ws-1', agentKind: 'server', tags: ['finance', 'office-hq'], registeredVia: `link:${l.link.id}` });
    // Use limit reached; links list shows it as no longer usable.
    expect((await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: l.token } }))).status).toBe(410);
    expect((await preload.GET(await req('GET', `/api/fleet/links/${l.link.id}/preload`, { as: admin }), P(l.link.id))).status).toBe(410);
    expect((await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: 'ufop_reg_nope' } }))).status).toBe(401);
    expect((await register.POST(await req('POST', '/api/agent/register', { body: {} }))).status).toBe(400);

    const l2 = await newLink(admin);
    expect((await linkById.DELETE(await req('DELETE', `/api/fleet/links/${l2.link.id}`, { as: admin }), P(l2.link.id))).status).toBe(200);
    expect((await linkById.DELETE(await req('DELETE', `/api/fleet/links/${l2.link.id}`, { as: admin }), P(l2.link.id))).status).toBe(404);
    expect(await json(await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: l2.token } })))).toMatchObject({ status: 410, body: { error: 'This registration link was revoked' } });
    // Expired links.
    const l3 = await newLink(admin, { expiresInHours: 1 });
    db().prepare('UPDATE registration_links SET expires_at = ? WHERE id = ?').run(new Date(Date.now() - 1000).toISOString(), l3.link.id);
    expect((await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: l3.token } }))).status).toBe(410);
    // Operators may enroll, auditors may not.
    expect((await links.POST(await req('POST', '/api/fleet/links', { as: makeUser('aud@x.org', 'read_only_auditor'), body: {} }))).status).toBe(403);
  });

  it('rate-limits enrollment and agent authentication failures per address', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const { token } = await newLink(admin);
    for (let i = 0; i < 20; i++) expect((await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: `ufop_reg_bad${i}` } }))).status).toBe(401);
    // Even the right token is refused while the address is locked out.
    expect((await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: token } }))).status).toBe(429);
    for (let i = 0; i < 20; i++) await hb(`ufop_agent_wrong${i}`);
    expect((await hb('ufop_agent_whatever')).status).toBe(429);
    // Claims: every start counts.
    for (let i = 0; i < 20; i++) expect((await claim.POST(await req('POST', '/api/agent/claim', { body: {}, headers: { 'x-forwarded-for': '198.51.100.1' } }))).status).toBe(201);
    expect((await claim.POST(await req('POST', '/api/agent/claim', { body: {}, headers: { 'x-forwarded-for': '198.51.100.1' } }))).status).toBe(429);
  });

  it('claims a machine by code and hands the token over exactly once', async () => {
    const op = makeUser('op@x.org', 'operator');
    const started = await json<{ claimId: string; code: string; secret: string }>(await claim.POST(await req('POST', '/api/agent/claim', { body: { info: { hostname: 'kiosk-7', os: 'macos', clientVersion: '2.1.0' } } })));
    expect(started.status).toBe(201);
    expect(started.body.code).toMatch(/^[A-Z2-9]{4}-[A-Z2-9]{4}$/);
    const poll = async (secret = started.body.secret) => json<{ status: string; agentToken?: string; machineId?: string }>(await claimPoll.POST(await req('POST', '/api/agent/claim/poll', { body: { claimId: started.body.claimId, secret } })));
    expect((await poll()).body).toEqual({ status: 'pending' });
    expect((await poll('ufop_claim_wrong')).status).toBe(404);
    expect((await claims.POST(await req('POST', '/api/fleet/claims', { as: op, body: { code: 'ZZZZ-ZZZZ' } }))).status).toBe(404);
    // Codes are accepted in any case, with or without the dash.
    const c = await json<{ machine: { id: string; name: string; tags: string[] } }>(await claims.POST(await req('POST', '/api/fleet/claims', { as: op, body: { code: started.body.code.replace('-', '').toLowerCase(), name: 'Kiosk 7', tags: ['lobby'] } })));
    expect(c.status).toBe(201);
    expect(c.body.machine).toMatchObject({ name: 'Kiosk 7', tags: ['lobby'] });
    expect((await claims.POST(await req('POST', '/api/fleet/claims', { as: op, body: { code: started.body.code } }))).status).toBe(404);
    const got = await poll();
    expect(got.body).toMatchObject({ status: 'collected', machineId: c.body.machine.id });
    expect(got.body.agentToken).toMatch(/^ufop_agent_/);
    expect((await poll()).body).toEqual({ status: 'collected' });
    expect((await hb(got.body.agentToken!)).status).toBe(200);
    expect(getMachine(c.body.machine.id)).toMatchObject({ hostname: 'kiosk-7', os: 'macos', clientVersion: '2.1.0' });
    // Wrong codes are rate-limited per user.
    for (let i = 0; i < 20; i++) await claims.POST(await req('POST', '/api/fleet/claims', { as: op, body: { code: `AAAA-AAA${i}` } }));
    expect((await claims.POST(await req('POST', '/api/fleet/claims', { as: op, body: { code: 'AAAA-AAAA' } }))).status).toBe(429);
  });
});

describe('agent protocol', () => {
  it('heartbeats update client info, jobs and schedules', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = await enroll(admin, 'srv');
    const r = await json<{ machineId: string; status: string; commands: unknown[]; policySetVersion: string; policiesChanged: boolean; heartbeatSeconds: number }>(
      await hb(m.token, { info: { clientVersion: '1.2.0', osVersion: '24.04' }, heartbeatSeconds: 5, jobs: [{ id: 'job1', name: 'Documents', scheduleIntervalMinutes: 60, lastResult: 'success', lastRunAt: null }, { name: 'no id' }] }),
    );
    expect(r.body).toMatchObject({ machineId: m.id, status: 'active', commands: [], policySetVersion: '', policiesChanged: false });
    const mm = getMachine(m.id)!;
    expect(mm).toMatchObject({ clientVersion: '1.2.0', osVersion: '24.04', heartbeatSeconds: 15, connectivity: 'online', lastIp: '203.0.113.7' });
    expect(mm.jobs.map((j) => j.id)).toEqual(['job1']);
    expect(listBackups(['org_default'])).toMatchObject([{ backupId: 'job1', name: 'Documents', scheduleIntervalMinutes: 60, lastRunAt: null }]);
  });

  it('stores reports and rolls up backups', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = await enroll(admin, 'srv');
    const first = await sendReport(m.token, { backup_id: 'job1', name: 'Docs', operation: 'backup', result: 'success', duration: '12s', finished_at: '2026-10-20T10:00:00Z', data: { files_examined: 100, files_added: 3, files_changed: 2, files_deleted: 1, bytes_uploaded: 4096, warnings: [], errors: [] }, stats: { version_count: 4, backend_size: 5000, source_size: 9000, schedule_interval_minutes: 1440 } });
    expect(first.status).toBe(201);
    const rid = ((await first.json()) as { id: string }).id;
    // A failed run without stats keeps the sizes and the last success.
    expect((await sendReport(m.token, { backup_id: 'job1', name: 'Docs', result: 'failed', finished_at: '2026-10-21T10:00:00Z', data: { errors: ['disk full', 'io'] } })).status).toBe(201);
    expect(listBackups(['org_default'])).toEqual([
      { orgId: 'org_default', machineId: m.id, backupId: 'job1', name: 'Docs', lastStatus: 'error', lastRunAt: '2026-10-21T10:00:00.000Z', lastSuccessAt: '2026-10-20T10:00:00.000Z', scheduleIntervalMinutes: 1440, sourceSize: 9000, backendSize: 5000, versionCount: 4 },
    ]);
    const one = await json<{ report: Record<string, unknown>; raw: { stats: unknown } }>(await reports.GET(await req('GET', `/api/fleet/reports?id=${rid}`, { as: admin })));
    expect(one.body.report).toMatchObject({ durationMs: 12000, filesExamined: 100, filesAdded: 3, filesModified: 2, filesDeleted: 1, bytesUploaded: 4096, versionCount: 4, backendSize: 5000 });
    expect(one.body.raw.stats).toMatchObject({ version_count: 4 });
    expect((await json<{ reports: { status: string }[] }>(await reports.GET(await req('GET', '/api/fleet/reports?status=error', { as: admin })))).body.reports).toHaveLength(1);
    // Restores are stored but do not touch the rollup.
    expect((await sendReport(m.token, { backup_id: 'job1', operation: 'restore', result: 'success' })).status).toBe(201);
    expect(listBackups(['org_default'])[0].lastStatus).toBe('error');
    expect((await sendReport(m.token, { result: 'success' })).status).toBe(422);
    const bad = await agentReport.POST(new NextRequest(new URL('http://localhost:3001/api/agent/report'), { method: 'POST', headers: { ...bearer(m.token), 'content-type': 'application/json' }, body: '{not json' }));
    expect(bad.status).toBe(400);
    expect(getMachine(m.id)!.lastReportAt).toBeTruthy();
  });

  it('accepts plain report URLs (JSON and form-encoded) keyed by a link token', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const { token } = await newLink(admin, { defaultTags: ['remote'] });
    const url = `/api/ingest/${encodeURIComponent(token)}`;
    const p = { params: Promise.resolve({ token: encodeURIComponent(token) }) };
    const a = await json<{ id: string; machineId: string }>(await ingest.POST(rawReq(url, JSON.stringify({ machine: 'nas-01', backup_id: 'b1', name: 'Photos', result: 'success', stats: { version_count: 2, backend_size: 10 } }), 'application/json'), p));
    expect(a.status).toBe(201);
    const form = new URLSearchParams({ data: JSON.stringify({ machine: 'NAS-01', backup_id: 'b1', name: 'Photos', result: 'warning', data: { warnings: ['locked file'] } }) }).toString();
    const b = await json<{ machineId: string }>(await ingest.POST(rawReq(url, form, 'application/x-www-form-urlencoded'), p));
    expect(b.status).toBe(201);
    // Same machine (name match is case-insensitive), created as a report-only client.
    expect(b.body.machineId).toBe(a.body.machineId);
    expect(getMachine(a.body.machineId)).toMatchObject({ agentKind: 'report', tags: ['remote'], name: 'nas-01' });
    expect(listBackups(['org_default'])[0]).toMatchObject({ lastStatus: 'warning', versionCount: 2, backendSize: 10 });
    expect((await ingest.POST(rawReq(url, '{oops', 'application/json'), p)).status).toBe(400);
    expect((await ingest.POST(rawReq(url, JSON.stringify({ machine: 'x' }), 'application/json'), p)).status).toBe(422);
    const bogus = { params: Promise.resolve({ token: 'ufop_reg_bogus' }) };
    expect((await ingest.POST(rawReq('/api/ingest/ufop_reg_bogus', JSON.stringify({ backup_id: 'b' }), 'application/json'), bogus)).status).toBe(401);
    // Suspended report machines are refused.
    db().prepare("UPDATE machines SET status = 'suspended' WHERE id = ?").run(a.body.machineId);
    expect((await ingest.POST(rawReq(url, JSON.stringify({ machine: 'nas-01', backup_id: 'b1' }), 'application/json'), p)).status).toBe(423);
    // Unknown keys count towards the limiter.
    for (let i = 0; i < 20; i++) await ingest.POST(rawReq('/api/ingest/x', JSON.stringify({ backup_id: 'b' }), 'application/json'), bogus);
    expect((await ingest.POST(rawReq(url, JSON.stringify({ machine: 'other', backup_id: 'b1' }), 'application/json'), p)).status).toBe(429);
  });

  it('rotates tokens, honours suspension and revocation, and unregisters', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = await enroll(admin, 'srv');
    const rot = await json<{ agentToken: string }>(await rotate.POST(await req('POST', '/api/agent/rotate', { headers: bearer(m.token) })));
    expect(rot.body.agentToken).toMatch(/^ufop_agent_/);
    expect((await hb(m.token)).status).toBe(401);
    expect((await hb(rot.body.agentToken)).status).toBe(200);
    // Admin-initiated rotation goes through the agent.
    await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { action: 'rotate_token' } }), P(m.id));
    const cmds = (await json<{ commands: { kind: string }[] }>(await hb(rot.body.agentToken))).body.commands;
    expect(cmds.map((c) => c.kind)).toEqual(['rotate_token']);
    // Suspension: 423, resumption restores access.
    await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { action: 'suspend' } }), P(m.id));
    expect((await hb(rot.body.agentToken)).status).toBe(423);
    expect((await commands.POST(await req('POST', `/api/fleet/machines/${m.id}/commands`, { as: admin, body: { kind: 'apply_policies' } }), P(m.id))).status).toBe(409);
    await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { action: 'resume' } }), P(m.id));
    expect((await hb(rot.body.agentToken)).status).toBe(200);
    expect((await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { action: 'explode' } }), P(m.id))).status).toBe(400);
    // Revocation.
    await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { action: 'revoke' } }), P(m.id));
    expect((await hb(rot.body.agentToken)).status).toBe(401);
    expect((await sendReport(rot.body.agentToken, { backup_id: 'x' })).status).toBe(401);
    // Resuming a revoked machine does not bring it back.
    await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { action: 'resume' } }), P(m.id));
    expect(getMachine(m.id)!.status).toBe('revoked');

    const m2 = await enroll(admin, 'leaving');
    expect((await unregister.POST(await req('POST', '/api/agent/unregister', { headers: bearer(m2.token) }))).status).toBe(200);
    expect((await hb(m2.token)).status).toBe(401);
    expect(getMachine(m2.id)!.status).toBe('revoked');
    // Revoked machines leave the list and free their seat.
    expect((await json<{ machines: unknown[] }>(await machines.GET(await req('GET', '/api/fleet/machines', { as: admin })))).body.machines).toHaveLength(0);
    expect((await machineById.DELETE(await req('DELETE', `/api/fleet/machines/${m.id}`, { as: admin }), P(m.id))).status).toBe(200);
    expect(getMachine(m.id)).toBeNull();
    expect(listAudit({ limit: 500 }).entries.map((e) => e.eventType)).toEqual(expect.arrayContaining(['machine.rotate_token', 'machine.suspend', 'machine.resume', 'machine.revoke', 'machine.unregister', 'machine.delete']));
  });
});

describe('tags and dashboards', () => {
  it('normalizes tags, stores colors and filters by tag', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const a = await enroll(admin, 'a', ['Office HQ', 'linux']);
    await enroll(admin, 'b', ['linux']);
    const put = await json<{ colors: Record<string, string> }>(await tags.PUT(await req('PUT', '/api/fleet/tags', { as: admin, body: { tag: 'Office HQ', color: '#FF0000' } })));
    expect(put.body.colors).toEqual({ 'office-hq': '#ff0000' });
    expect((await tags.PUT(await req('PUT', '/api/fleet/tags', { as: admin, body: { tag: 'x', color: 'red' } }))).status).toBe(400);
    expect((await tags.PUT(await req('PUT', '/api/fleet/tags', { as: admin, body: { tag: '!!!', color: '#000000' } }))).status).toBe(400);
    const list = await json<{ tags: { tag: string; count: number; color: string | null }[] }>(await tags.GET(await req('GET', '/api/fleet/tags', { as: admin })));
    expect(list.body.tags).toEqual([{ tag: 'linux', count: 2, color: null }, { tag: 'office-hq', count: 1, color: '#ff0000' }]);
    const filtered = await json<{ machines: { id: string }[]; tagColors: Record<string, string> }>(await machines.GET(await req('GET', '/api/fleet/machines?tag=office-hq&tag=linux', { as: admin })));
    expect(filtered.body.machines.map((m) => m.id)).toEqual([a.id]);
    expect(filtered.body.tagColors).toEqual({ 'office-hq': '#ff0000' });
    // Editing tags on a machine normalizes them too.
    const upd = await json<{ machine: { tags: string[] } }>(await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${a.id}`, { as: admin, body: { tags: ['Web Servers', 'web servers', ''] } }), P(a.id)));
    expect(upd.body.machine.tags).toEqual(['web-servers']);
    expect((await tags.PUT(await req('PUT', '/api/fleet/tags', { as: admin, body: { tag: 'office-hq', color: null } }))).status).toBe(200);
  });

  it('aggregates machines, backups, runs and storage', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const a = await enroll(admin, 'a', ['prod']);
    const b = await enroll(admin, 'b');
    await enroll(admin, 'never-seen');
    db().prepare('UPDATE machines SET last_seen_at = NULL WHERE name = ?').run('never-seen');
    db().prepare('UPDATE machines SET last_seen_at = ? WHERE id = ?').run(new Date(Date.now() - 3600_000).toISOString(), b.id);
    await sendReport(a.token, { backup_id: 'j1', name: 'Big', result: 'success', data: { bytes_uploaded: 100 }, stats: { backend_size: 9000, source_size: 20000, version_count: 3 } });
    await sendReport(a.token, { backup_id: 'j2', name: 'Small', result: 'warning', data: { bytes_uploaded: 50 }, stats: { backend_size: 100, source_size: 200, version_count: 1 } });
    await sendReport(b.token, { backup_id: 'j1', name: 'Other', result: 'error' });
    await hb(a.token, { jobs: [{ id: 'j3', name: 'Never ran', scheduleIntervalMinutes: 60 }] });
    // Re-mark b as stale (its own requests refreshed last_seen_at).
    db().prepare('UPDATE machines SET last_seen_at = ? WHERE id = ?').run(new Date(Date.now() - 3600_000).toISOString(), b.id);
    const d = await json<{ machines: Record<string, number>; backups: Record<string, number>; runs: Record<string, number>; storageHistory: { backendSize: number; runs: number; failures: number }[]; largestBackups: { name: string; machineName: string }[]; license: { used: number } }>(await dashboard.GET(await req('GET', '/api/fleet/dashboard', { as: admin })));
    expect(d.body.machines).toEqual({ total: 3, online: 1, stale: 1, offline: 0, never: 1, suspended: 0 });
    expect(d.body.backups).toEqual({ total: 4, success: 1, warning: 1, error: 1, neverRun: 1, sourceSize: 20200, backendSize: 9100, versions: 4 });
    expect(d.body.runs).toEqual({ total: 3, failed: 1, warnings: 1, bytesUploaded: 150 });
    expect(d.body.storageHistory).toHaveLength(1);
    expect(d.body.storageHistory[0]).toMatchObject({ backendSize: 9100, runs: 3, failures: 1 });
    expect(d.body.largestBackups[0]).toMatchObject({ name: 'Big', machineName: 'a' });
    expect(d.body.license.used).toBe(3);
    const tagged = await json<{ machines: { total: number }; backups: { total: number } }>(await dashboard.GET(await req('GET', '/api/fleet/dashboard?tag=prod', { as: admin })));
    expect(tagged.body.machines.total).toBe(1);
    expect(tagged.body.backups.total).toBe(3);
    // Sub-organizations are included on request.
    const child = createOrg({ name: 'Child', parentId: 'org_default' });
    makeUser('c@x.org', 'org_admin', child.id);
    await enroll(makeUser('c2@x.org', 'org_admin', child.id), 'child-srv');
    expect((await json<{ machines: { total: number } }>(await dashboard.GET(await req('GET', '/api/fleet/dashboard?includeChildren=1', { as: admin })))).body.machines.total).toBe(4);
  });
});

describe('remote commands', () => {
  it('queues allowed commands, delivers them on heartbeat and records results', async () => {
    const op = makeUser('op@x.org', 'operator');
    const m = await enroll(op, 'srv');
    const other = await enroll(op, 'other');
    const send = async (body: unknown) => json<{ command: { id: string; status: string } }>(await commands.POST(await req('POST', `/api/fleet/machines/${m.id}/commands`, { as: op, body }), P(m.id)));
    expect((await send({ kind: 'rpc', payload: { command: 'backup_export_config_with_secrets' } })).status).toBe(403);
    expect((await send({ kind: 'rpc', payload: {} })).status).toBe(403);
    expect((await send({ kind: 'shell', payload: { cmd: 'rm -rf /' } })).status).toBe(400);
    expect((await send({ kind: 'run_backup', payload: {} })).status).toBe(400);
    const c1 = await send({ kind: 'rpc', payload: { command: 'backup_list_jobs', args: {} } });
    expect(c1.status).toBe(201);
    const c2 = await send({ kind: 'run_backup', payload: { jobId: 'job1' } });
    const c3 = await send({ kind: 'apply_policies' });
    // Cancel one before delivery.
    expect((await commands.DELETE(await req('DELETE', `/api/fleet/machines/${m.id}/commands?commandId=${c3.body.command.id}`, { as: op }), P(m.id))).status).toBe(200);
    expect(getCommand(c3.body.command.id)!.status).toBe('cancelled');

    const delivered = (await json<{ commands: { id: string; kind: string; payload: Record<string, unknown> }[] }>(await hb(m.token))).body.commands;
    expect(delivered.map((c) => [c.kind, c.payload])).toEqual([['rpc', { command: 'backup_list_jobs', args: {} }], ['run_backup', { jobId: 'job1' }]]);
    expect((await json<{ commands: unknown[] }>(await hb(m.token))).body.commands).toEqual([]);
    expect(getCommand(c1.body.command.id)!.status).toBe('delivered');
    // Cancelling is only for queued commands.
    expect((await commands.DELETE(await req('DELETE', `/api/fleet/machines/${m.id}/commands?commandId=${c1.body.command.id}`, { as: op }), P(m.id))).status).toBe(404);

    // Another machine cannot answer for this one.
    expect((await agentCommand.POST(await req('POST', `/api/agent/commands/${c1.body.command.id}`, { headers: bearer(other.token), body: { ok: true } }), P(c1.body.command.id))).status).toBe(404);
    expect((await agentCommand.POST(await req('POST', `/api/agent/commands/${c1.body.command.id}`, { headers: bearer(m.token), body: { ok: true, result: { jobs: [{ id: 'job1' }] } } }), P(c1.body.command.id))).status).toBe(200);
    expect((await agentCommand.POST(await req('POST', `/api/agent/commands/${c1.body.command.id}`, { headers: bearer(m.token), body: { ok: false } }), P(c1.body.command.id))).status).toBe(404);
    await agentCommand.POST(await req('POST', `/api/agent/commands/${c2.body.command.id}`, { headers: bearer(m.token), body: { ok: false, result: { error: 'busy' } } }), P(c2.body.command.id));
    const one = await json<{ command: { status: string; result: unknown } }>(await commands.GET(await req('GET', `/api/fleet/machines/${m.id}/commands?commandId=${c1.body.command.id}`, { as: op }), P(m.id)));
    expect(one.body.command).toMatchObject({ status: 'succeeded', result: { jobs: [{ id: 'job1' }] } });
    // A command id from another machine is not shown under this one.
    expect((await commands.GET(await req('GET', `/api/fleet/machines/${other.id}/commands?commandId=${c1.body.command.id}`, { as: op }), P(other.id))).status).toBe(404);
    const all = await json<{ commands: { status: string }[] }>(await commands.GET(await req('GET', `/api/fleet/machines/${m.id}/commands`, { as: op }), P(m.id)));
    expect(all.body.commands.map((c) => c.status).sort()).toEqual(['cancelled', 'failed', 'succeeded']);
    // Expired commands are not delivered.
    const c4 = await send({ kind: 'apply_policies' });
    db().prepare('UPDATE agent_commands SET expires_at = ? WHERE id = ?').run(new Date(Date.now() - 1000).toISOString(), c4.body.command.id);
    expect((await json<{ commands: unknown[] }>(await hb(m.token))).body.commands).toEqual([]);
    expect(getCommand(c4.body.command.id)!.status).toBe('expired');
    // Auditors cannot run remote commands.
    expect((await commands.POST(await req('POST', `/api/fleet/machines/${m.id}/commands`, { as: makeUser('aud@x.org', 'read_only_auditor'), body: { kind: 'apply_policies' } }), P(m.id))).status).toBe(403);
    const types = listAudit({}).entries.map((e) => e.eventType);
    expect(types.filter((t) => t === 'remote.command')).toHaveLength(4);
    // Regression: cancelling a queued command was not audited.
    expect(types).toContain('remote.command_cancel');
  });
});

describe('fleet policies and central credentials', () => {
  const job = (cred: string) => ({ name: 'Managed docs', sources: ['/home'], destination: { type: 's3', bucket: 'b', credential_id: `central:${cred}` }, encrypted: true, passphraseCredentialId: `central:${cred}`, schedule: { every_minutes: 60 } });

  it('keeps secrets away from the UI and delivers them only to targeted agents', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const sec = makeUser('sec@x.org', 'security_admin');
    const cr = await json<{ credential: Record<string, unknown> }>(await credentials.POST(await req('POST', '/api/fleet/credentials', { as: sec, body: { name: 'S3 key', kind: 'password', secret: 'sup3r-s3cret-value' } })));
    expect(cr.status).toBe(201);
    const credId = cr.body.credential.id as string;
    expect(JSON.stringify(cr.body)).not.toContain('sup3r-s3cret-value');
    expect(Object.keys(cr.body.credential)).not.toContain('ciphertext');
    // Sealed at rest.
    const stored = db().prepare('SELECT ciphertext FROM central_credentials WHERE id = ?').get(credId) as { ciphertext: string };
    expect(stored.ciphertext).toMatch(/^v1\./);
    expect(stored.ciphertext).not.toContain('sup3r');
    expect((await credentials.POST(await req('POST', '/api/fleet/credentials', { as: sec, body: { name: 's3 KEY', secret: 'x' } }))).status).toBe(409);
    expect((await credentials.POST(await req('POST', '/api/fleet/credentials', { as: makeUser('op@x.org', 'operator'), body: { name: 'n', secret: 'x' } }))).status).toBe(403);

    const inScope = await enroll(admin, 'in-scope', ['managed']);
    const outOfScope = await enroll(admin, 'out-of-scope');
    const otherOrg = createOrg({ name: 'Other' });
    const foreign = await enroll(makeUser('o@other.org', 'org_admin', otherOrg.id), 'foreign', ['managed']);

    // Validation: unknown fields, missing passphrase credential, credentials of other organizations.
    const post = async (body: unknown, as = admin) => json<{ policy: { id: string; version: number; credentialIds: string[] } }>(await fleetPolicies.POST(await req('POST', '/api/fleet/policies', { as, body })));
    expect((await post({ name: 'p', target: { tags: ['managed'] }, job: { ...job(credId), id: 'hijack' } })).status).toBe(400);
    expect((await post({ name: 'p', target: { tags: ['managed'] }, job: { name: 'j', sources: ['/'], destination: { type: 'local' } } })).status).toBe(400);
    expect((await post({ name: 'p', target: { tags: ['managed'] }, job: job(credId) }, makeUser('oa@other.org', 'org_admin', otherOrg.id))).status).toBe(400);

    const created = await post({ name: 'Docs', target: { tags: ['Managed'] }, job: job(credId) });
    expect(created.status).toBe(201);
    const pol = created.body.policy;
    expect(pol.credentialIds).toEqual([credId]);
    // A policy in the other organization targeting the same tag.
    await post({ name: 'Foreign', target: { all: true }, job: { name: 'f', sources: ['/'], destination: { type: 'local', path: '/x' }, encrypted: false } }, makeUser('ob@other.org', 'org_admin', otherOrg.id));

    // UI surfaces never contain the secret.
    const uiResponses = [
      await credentials.GET(await req('GET', '/api/fleet/credentials', { as: admin })),
      await fleetPolicies.GET(await req('GET', '/api/fleet/policies', { as: admin })),
      await machineById.GET(await req('GET', `/api/fleet/machines/${inScope.id}`, { as: admin }), P(inScope.id)),
      await machines.GET(await req('GET', '/api/fleet/machines', { as: admin })),
    ];
    for (const r of uiResponses) expect(await r.text()).not.toContain('sup3r-s3cret-value');

    // The heartbeat announces the policy set; only the targeted agent receives the secret.
    const beat = await json<{ policySetVersion: string; policiesChanged: boolean }>(await hb(inScope.token));
    const set1 = `cred:${credId}:1,${pol.id}:1`;
    expect(beat.body).toMatchObject({ policySetVersion: set1, policiesChanged: true });
    expect((await json<{ policiesChanged: boolean }>(await hb(inScope.token, { policySetVersion: set1 }))).body.policiesChanged).toBe(false);
    const got = await agentPolicies.GET(await req('GET', '/api/agent/policies', { headers: bearer(inScope.token) }));
    expect(got.headers.get('cache-control')).toBe('no-store');
    const body = (await got.json()) as { policies: { id: string; version: number; job: Record<string, unknown> }[]; credentials: { credentialId: string; secret: string }[] };
    expect(body.policies).toMatchObject([{ id: pol.id, version: 1, job: { name: 'Managed docs' } }]);
    expect(body.credentials).toMatchObject([{ credentialId: `central:${credId}`, name: 'S3 key', secret: 'sup3r-s3cret-value', version: 1 }]);
    for (const t of [outOfScope.token, foreign.token]) {
      const r = await (await agentPolicies.GET(await req('GET', '/api/agent/policies', { headers: bearer(t) }))).text();
      expect(r).not.toContain('sup3r-s3cret-value');
      expect(r).not.toContain(pol.id);
    }
    expect((await agentPolicies.GET(await req('GET', '/api/agent/policies'))).status).toBe(401);
    expect(listAudit({ eventType: 'credential.deliver' }).total).toBe(1);

    // The agent reports what it applied; compliance follows.
    await applied.POST(await req('POST', '/api/agent/policies/applied', { headers: bearer(inScope.token), body: { applied: { [pol.id]: 1, [`credential:${credId}`]: 1, bogus: 'x' }, errors: {} } }));
    const comp = await json<{ policies: { id: string; targeted: number; upToDate: number }[] }>(await fleetPolicies.GET(await req('GET', '/api/fleet/policies', { as: admin })));
    expect(comp.body.policies.find((p) => p.id === pol.id)).toMatchObject({ targeted: 1, upToDate: 1 });
    expect(getMachine(inScope.id)!.appliedPolicies).toEqual({ [pol.id]: 1, [`credential:${credId}`]: 1 });
    type CredView = { id: string; version: number; propagation: { targeted: number; upToDate: number; pending: { id: string; version: number | null }[] } };
    const credView = async () => (await json<{ credentials: CredView[] }>(await credentials.GET(await req('GET', '/api/fleet/credentials', { as: admin })))).body.credentials.find((c) => c.id === credId)!;
    expect(await credView()).toMatchObject({ version: 1, propagation: { targeted: 1, upToDate: 1, pending: [] } });

    // Rotation: a new credential version; the policy itself is unchanged, the
    // policy set version moves so the agent fetches the new secret.
    const rot = await json<{ credential: { version: number } }>(await credentialById.PUT(await req('PUT', `/api/fleet/credentials/${credId}`, { as: sec, body: { secret: 'rotated-value' } }), P(credId)));
    expect(rot.status).toBe(200);
    expect(rot.body.credential.version).toBe(2);
    expect((await json<{ policiesChanged: boolean; policySetVersion: string }>(await hb(inScope.token, { policySetVersion: set1 }))).body).toMatchObject({ policiesChanged: true, policySetVersion: `cred:${credId}:2,${pol.id}:1` });
    const after = (await (await agentPolicies.GET(await req('GET', '/api/agent/policies', { headers: bearer(inScope.token) }))).json()) as { policies: { version: number }[]; credentials: { secret: string; version: number }[] };
    expect(after.credentials[0]).toMatchObject({ secret: 'rotated-value', version: 2 });
    expect(after.policies[0].version).toBe(1);
    const comp2 = await json<{ policies: { id: string; upToDate: number }[] }>(await fleetPolicies.GET(await req('GET', '/api/fleet/policies', { as: admin })));
    expect(comp2.body.policies.find((p) => p.id === pol.id)).toMatchObject({ upToDate: 1 });
    expect(await credView()).toMatchObject({ version: 2, propagation: { targeted: 1, upToDate: 0, pending: [{ id: inScope.id, version: 1 }] } });
    await applied.POST(await req('POST', '/api/agent/policies/applied', { headers: bearer(inScope.token), body: { applied: { [pol.id]: 1, [`credential:${credId}`]: 2 } } }));
    expect((await credView()).propagation).toMatchObject({ upToDate: 1, pending: [] });
    const hist = await json<{ history: { version: number; createdBy: string }[] }>(await credentialById.GET(await req('GET', `/api/fleet/credentials/${credId}`, { as: admin }), P(credId)));
    expect(hist.body.history.map((h) => [h.version, h.createdBy])).toEqual([[2, sec.email], [1, sec.email]]);
    expect(JSON.stringify(hist.body)).not.toContain('rotated-value');
    expect(listAudit({ eventType: 'credential.rotate' }).entries[0].description).toContain('version 2');

    // Failures to apply are audited.
    await applied.POST(await req('POST', '/api/agent/policies/applied', { headers: bearer(inScope.token), body: { applied: {}, errors: { [pol.id]: 'destination unreachable' } } }));
    expect(listAudit({ eventType: 'policy.apply_failed' }).entries[0].description).toContain('destination unreachable');

    // A credential in use cannot be deleted; a disabled policy is no longer delivered.
    expect((await credentialById.DELETE(await req('DELETE', `/api/fleet/credentials/${credId}`, { as: sec }), P(credId))).status).toBe(409);
    expect((await fleetPolicyById.PUT(await req('PUT', `/api/fleet/policies/${pol.id}`, { as: admin, body: { enabled: false } }), P(pol.id))).status).toBe(200);
    const off = await (await agentPolicies.GET(await req('GET', '/api/agent/policies', { headers: bearer(inScope.token) }))).json() as { policies: unknown[]; credentials: unknown[] };
    expect(off).toMatchObject({ policies: [], credentials: [] });
    expect((await fleetPolicyById.DELETE(await req('DELETE', `/api/fleet/policies/${pol.id}`, { as: admin }), P(pol.id))).status).toBe(200);
    expect((await credentialById.DELETE(await req('DELETE', `/api/fleet/credentials/${credId}`, { as: sec }), P(credId))).status).toBe(200);
    expect((await credentialById.PUT(await req('PUT', `/api/fleet/credentials/${credId}`, { as: sec, body: { secret: 'x' } }), P(credId))).status).toBe(404);
    const types = listAudit({ limit: 500 }).entries.map((e) => e.eventType);
    expect(types).toEqual(expect.arrayContaining(['credential.create', 'credential.rotate', 'credential.delete', 'credential.deliver', 'fleet.policy_create', 'fleet.policy_update', 'fleet.policy_delete', 'policy.apply_failed']));
    // The audit trail never carries the secret.
    expect(JSON.stringify(listAudit({ limit: 500 }).entries)).not.toMatch(/sup3r-s3cret-value|rotated-value/);
    expect(verifyAudit()).toMatchObject({ ok: true });
  });
});

describe('audit trail of fleet actions', () => {
  it('records enrollment and management events and still verifies', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const l = await newLink(admin);
    await preload.GET(await req('GET', `/api/fleet/links/${l.link.id}/preload`, { as: admin }), P(l.link.id));
    const m = await enroll(admin, 'srv');
    await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m.id}`, { as: admin, body: { name: 'renamed' } }), P(m.id));
    await tags.PUT(await req('PUT', '/api/fleet/tags', { as: admin, body: { tag: 't', color: '#123456' } }));
    await rotate.POST(await req('POST', '/api/agent/rotate', { headers: bearer(m.token) }));
    const c = await claim.POST(await req('POST', '/api/agent/claim', { body: {} }));
    await claims.POST(await req('POST', '/api/fleet/claims', { as: admin, body: { code: ((await c.json()) as { code: string }).code } }));
    await linkById.DELETE(await req('DELETE', `/api/fleet/links/${l.link.id}`, { as: admin }), P(l.link.id));
    const types = listAudit({}).entries.map((e) => e.eventType);
    expect(types).toEqual(expect.arrayContaining(['fleet.link_create', 'fleet.preload_download', 'machine.register', 'machine.update', 'fleet.tag_color', 'machine.token_rotate', 'machine.claim', 'fleet.link_revoke']));
    expect(verifyAudit()).toMatchObject({ ok: true });
  });

  it('stores diagnostics bundles from agents, on demand and by command', async () => {
    const admin = makeUser('diag-admin@x.org', 'org_admin');
    const viewer = makeUser('diag-auditor@x.org', 'read_only_auditor');
    const owner = makeUser('diag-user@x.org', 'user');
    const m = await enroll(admin, 'diag-box');
    const zip = Buffer.concat([Buffer.from('PK\x03\x04'), Buffer.alloc(32, 1)]);
    const upload = async (token: string, body: unknown) => agentDiagnostics.POST(await req('POST', '/api/agent/diagnostics', { body, headers: bearer(token) }));

    expect((await upload('ufop_agent_nope', { bundleBase64: zip.toString('base64') })).status).toBe(401);
    expect((await upload(m.token, {})).status).toBe(400);
    expect((await upload(m.token, { bundleBase64: Buffer.from('not a zip').toString('base64') })).status).toBe(422);
    expect((await upload(m.token, { bundleBase64: Buffer.alloc(MAX_DIAGNOSTICS_BYTES + 10, 0x50).toString('base64') })).status).toBe(413);
    const ok = await json<{ id: string }>(await upload(m.token, { fileName: '../../evil name.zip', summary: { appVersion: '1.2.3', os: 'linux' }, bundleBase64: zip.toString('base64') }));
    expect(ok.status).toBe(201);

    const list = await json<{ diagnostics: { id: string; fileName: string; size: number; summary: Record<string, unknown> }[] }>(
      await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics`, { as: viewer }), P(m.id)),
    );
    expect(list.body.diagnostics).toHaveLength(1);
    expect(list.body.diagnostics[0]).toMatchObject({ id: ok.body.id, fileName: '.._.._evil_name.zip', size: zip.length, summary: { appVersion: '1.2.3' } });

    const dl = await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics?download=${ok.body.id}`, { as: viewer }), P(m.id));
    expect(dl.status).toBe(200);
    expect(dl.headers.get('content-type')).toBe('application/zip');
    expect(Buffer.from(await dl.arrayBuffer()).equals(zip)).toBe(true);
    expect(listAudit({}).entries.some((e) => e.eventType === 'fleet.diagnostics_download')).toBe(true);

    // Admins request a bundle through the command queue.
    expect((await commands.POST(await req('POST', `/api/fleet/machines/${m.id}/commands`, { as: admin, body: { kind: 'collect_diagnostics' } }), P(m.id))).status).toBe(201);
    const cmds = (await json<{ commands: { kind: string }[] }>(await hb(m.token))).body.commands;
    expect(cmds.map((c) => c.kind)).toEqual(['collect_diagnostics']);

    // Only the newest bundles are kept.
    for (let i = 0; i < KEEP_PER_MACHINE + 2; i++) await upload(m.token, { fileName: `d${i}.zip`, bundleBase64: zip.toString('base64') });
    const after = (await json<{ diagnostics: { fileName: string }[] }>(await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics`, { as: admin }), P(m.id)))).body.diagnostics;
    expect(after).toHaveLength(KEEP_PER_MACHINE);
    expect(after[0].fileName).toBe(`d${KEEP_PER_MACHINE + 1}.zip`);

    // Deleting needs fleet.write.
    expect((await machineDiagnostics.DELETE(await req('DELETE', `/api/fleet/machines/${m.id}/diagnostics?id=x`, { as: viewer }), P(m.id))).status).toBe(403);
    const first = (await json<{ diagnostics: { id: string }[] }>(await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics`, { as: admin }), P(m.id)))).body.diagnostics[0].id;
    expect((await machineDiagnostics.DELETE(await req('DELETE', `/api/fleet/machines/${m.id}/diagnostics?id=${first}`, { as: admin }), P(m.id))).status).toBe(200);
    expect((await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics?download=${first}`, { as: admin }), P(m.id))).status).toBe(404);

    // Machine owners without a grant and other organizations see nothing.
    expect((await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics`, { as: owner }), P(m.id))).status).toBe(404);
    const other = createOrg({ name: 'Diag Other' });
    const outsider = makeUser('diag-out@y.org', 'org_admin', other.id);
    expect((await machineDiagnostics.GET(await req('GET', `/api/fleet/machines/${m.id}/diagnostics`, { as: outsider }), P(m.id))).status).toBe(404);
  });

  it('enrolls a desktop by browser sign-in with a loopback redirect and PKCE', async () => {
    const admin = makeUser('connect-admin@x.org', 'org_admin');
    const viewer = makeUser('connect-viewer@x.org', 'read_only_auditor');
    const verifier = 'v'.repeat(64);
    const challenge = createHash('sha256').update(verifier).digest('base64url');
    const redirectUri = 'http://127.0.0.1:49152/callback';
    const approve = async (as: UserRecord, body: Record<string, unknown>) => fleetConnect.POST(await req('POST', '/api/fleet/connect', { as, body }));
    const exchange = async (body: Record<string, unknown>) => connectToken.POST(await req('POST', '/api/agent/connect/token', { body }));

    expect((await approve(viewer, { redirectUri, state: 's1', codeChallenge: challenge })).status).toBe(403);
    for (const bad of ['https://127.0.0.1:1/cb', 'http://evil.example:80/cb', 'http://127.0.0.1/cb', 'http://user:pw@127.0.0.1:5/cb']) {
      expect((await approve(admin, { redirectUri: bad, state: 's1', codeChallenge: challenge })).status).toBe(400);
    }
    expect((await approve(admin, { redirectUri, state: 's1', codeChallenge: 'short' })).status).toBe(400);

    const ok = await json<{ redirect: string }>(await approve(admin, { redirectUri, state: 's1', codeChallenge: challenge, name: 'Ana laptop', tags: ['Laptop'] }));
    expect(ok.status).toBe(200);
    const back = new URL(ok.body.redirect);
    expect(back.origin + back.pathname).toBe(redirectUri);
    expect(back.searchParams.get('state')).toBe('s1');
    const code = back.searchParams.get('code')!;
    expect(code).toMatch(/^ufop_connect_/);

    // Wrong verifier burns the code; it cannot be retried with the right one.
    expect((await exchange({ code, codeVerifier: 'x'.repeat(64), redirectUri, info: { hostname: 'ana' } })).status).toBe(401);
    expect((await exchange({ code, codeVerifier: verifier, redirectUri, info: { hostname: 'ana' } })).status).toBe(401);

    const again = new URL((await json<{ redirect: string }>(await approve(admin, { redirectUri, state: 's2', codeChallenge: challenge, name: 'Ana laptop', tags: ['Laptop'] }))).body.redirect).searchParams.get('code')!;
    expect((await exchange({ code: again, codeVerifier: verifier, redirectUri: 'http://127.0.0.1:1/other', info: {} })).status).toBe(401);
    const third = new URL((await json<{ redirect: string }>(await approve(admin, { redirectUri, state: 's3', codeChallenge: challenge, name: 'Ana laptop', tags: ['Laptop'] }))).body.redirect).searchParams.get('code')!;
    const reg = await json<{ machineId: string; agentToken: string }>(await exchange({ code: third, codeVerifier: verifier, redirectUri, info: { hostname: 'ana', os: 'macos', clientVersion: '1.0.0' }, tags: ['dev'] }));
    expect(reg.status).toBe(201);
    expect(reg.body.agentToken).toMatch(/^ufop_agent_/);
    const m = getMachine(reg.body.machineId)!;
    expect(m).toMatchObject({ name: 'Ana laptop', hostname: 'ana', registeredVia: 'sso:connect-admin@x.org' });
    expect(m.tags).toEqual(['dev', 'laptop']);
    expect((await hb(reg.body.agentToken)).status).toBe(200);
    expect((await exchange({ code: third, codeVerifier: verifier, redirectUri, info: {} })).status).toBe(401);
    expect(listAudit({}).entries.map((e) => e.eventType)).toEqual(expect.arrayContaining(['fleet.desktop_connect']));
  });

  it('sends signed-out page requests to sign-in and back to the same page', async () => {
    const r = await middleware(new NextRequest(new URL('http://localhost:3001/fleet/connect?redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcb&state=s')));
    const loc = new URL(r.headers.get('location')!);
    expect(loc.pathname).toBe('/login');
    expect(loc.searchParams.get('redirect')).toBe('/fleet/connect?redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcb&state=s');
    const api = await middleware(new NextRequest(new URL('http://localhost:3001/api/fleet/machines')));
    expect(api.status).toBe(401);
  });

  it('manages app settings for desktops and delivers them with the policies', async () => {
    const RULE = { trigger: { type: 'file_created', path: '~/Downloads' }, action: { type: 'move', destination: '~/Sorted' } };
    const admin = makeUser('ms-admin@x.org', 'org_admin');
    const auditor = makeUser('ms-auditor@x.org', 'read_only_auditor');
    const m = await enroll(admin, 'ms-desk');
    const put = async (as: UserRecord, body: unknown) => managedSettings.PUT(await req('PUT', '/api/fleet/managed-settings', { as, body }));
    const bundle = async () => (await json<{ policySetVersion: string; managedSettings: Record<string, unknown> | null }>(await agentPolicies.GET(await req('GET', '/api/agent/policies', { headers: bearer(m.token) })))).body;

    expect((await bundle()).managedSettings).toBeNull();
    const v0 = (await bundle()).policySetVersion;
    expect((await put(auditor, { ai: { enabled: false } })).status).toBe(403);
    for (const bad of [
      { updates: { channel: 'nightly' } },
      { updates: { blackout: { start: '8:00', end: '18:00', days: [] } } },
      { updates: { blackout: { start: '08:00', end: '08:00', days: [] } } },
      { updates: { blackout: { start: '08:00', end: '18:00', days: [7] } } },
      { proxy: { mode: 'manual', url: 'proxy:3128' } },
      { proxy: { mode: 'pac' } },
      { proxy: { mode: 'magic' } },
      { ai: { enabled: 'no' } },
      { transfers: { maxConcurrentRuns: 9 } },
      { transfers: { maxUploadKbps: -1 } },
      { transfers: { maxDownloadKbps: 1.5 } },
      { mounts: { cacheMode: 'everything' } },
      { mounts: { allowedProtocols: ['gopher'] } },
      { mounts: { maxCacheMb: 0 } },
      { mounts: { readOnly: 'yes' } },
      { workflows: { name: 'x' } },
      { workflows: [{ name: '', rule: RULE }] },
      { workflows: [{ name: 'Bad id', id: 'no spaces', rule: RULE }] },
      { workflows: [{ name: 'Not a rule', rule: { hello: 1 } }] },
      { workflows: [{ name: 'Broken', rule: '{not json' }] },
      { workflows: [{ name: 'Runs', rule: { ...RULE, action: { type: 'run_command', command: 'rm' } } }] },
      { workflows: [{ name: 'Nested', rule: { ...RULE, workflow: { nodes: [{ kind: { type: 'action', action: { type: 'execute_custom_command' } } }] } } }] },
      { workflows: [{ name: 'Same', rule: RULE }, { name: 'same', rule: RULE }] },
      { workflows: Array.from({ length: 51 }, (_, i) => ({ name: `w${i}`, rule: RULE })) },
      { backupTemplates: { name: 'x' } },
      { backupTemplates: [{ name: '' }] },
      { backupTemplates: [{ name: 'Bad', id: 'no spaces' }] },
      { backupTemplates: [{ name: 'List', settings: [] }] },
      { backupTemplates: [{ name: 'Broken', settings: '{not json' }] },
      { backupTemplates: [{ name: 'Scripts', settings: { scripts: { run_before: 'rm -rf /' } } }] },
      { backupTemplates: [{ name: 'Same' }, { name: 'same' }] },
      { notificationProfiles: { name: 'x' } },
      { notificationProfiles: [{ name: 'No channel' }] },
      { notificationProfiles: [{ name: 'Typeless', notification: { url: 'x' } }] },
      { notificationProfiles: [{ name: 'Local secret', notification: { type: 'email', credential_id: 'local-1' } }] },
      { notificationProfiles: [{ name: 'Plain header', notification: { type: 'webhook', headers: [{ name: 'X-Token', value: 'hunter2', secret: true }] } }] },
      { notificationProfiles: [{ name: 'Severity', minSeverity: 'info', notification: { type: 'slack' } }] },
      { notificationProfiles: [{ name: 'Dup', notification: { type: 'slack' } }, { name: 'dup', notification: { type: 'teams' } }] },
      { serverAccess: [] },
      { serverAccess: { users: [{ username: 'ops', passwordHash: 'plain-password', role: 'operator' }] } },
      { serverAccess: { users: [{ username: 'has space', passwordHash: '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g' }] } },
      { serverAccess: { users: [{ username: 'ops', passwordHash: '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g', role: 'root' }] } },
      { serverAccess: { users: [{ username: 'ops', passwordHash: '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g' }, { username: 'OPS', passwordHash: '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g' }] } },
      { serverAccess: { rules: [{ principal: 'ops', jobPattern: '*', permission: 'all' }] } },
      { serverAccess: { rules: [{ principal: '', jobPattern: '*', permission: 'read' }] } },
    ]) expect((await put(admin, bad)).status).toBe(400);

    const ok = await json<{ settings: { version: number } }>(
      await put(admin, { updates: { channel: 'beta', autoCheck: null, blackout: { start: '08:00', end: '18:00', days: [5, 1, 1] } }, proxy: { mode: 'pac', pacUrl: 'https://wpad/p.pac', noProxy: [' intranet ', ''] }, ai: { allowCloud: false }, transfers: { maxUploadKbps: 512, maxConcurrentRuns: 2 }, mounts: { allowedProtocols: ['S3', 'rclone', 's3'], readOnly: true, cacheMode: 'full', maxCacheMb: 2048, mountRoot: ' ~/Drives ' }, workflows: [{ name: 'Tidy Downloads!', rule: JSON.stringify(RULE) }, { id: 'archive', name: 'Archive', enabled: false, rule: RULE }], backupTemplates: [{ name: 'Org Standard', enforced: true, settings: JSON.stringify({ retention: { type: 'keep_count', count: 7 } }) }], notificationProfiles: [{ name: 'On-call pager', minSeverity: 'error', notification: JSON.stringify({ type: 'webhook', preset: 'pagerduty', url: 'https://events.pagerduty.com/v2/enqueue', credential_id: 'central:pd', headers: [{ name: 'X-Key', secret: true, credential_id: 'central:pdh' }] }) }], serverAccess: { users: [{ username: ' ops ', passwordHash: '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g', role: 'operator' }], rules: [{ principal: 'ops', jobPattern: 'prod-*', permission: 'run' }] } }),
    );
    expect(ok.status).toBe(200);
    expect(ok.body.settings.version).toBe(1);
    const b = await bundle();
    expect(b.managedSettings).toEqual({
      version: 1,
      updates: { channel: 'beta', autoCheck: null, blackout: { start: '08:00', end: '18:00', days: [1, 5] } },
      proxy: { mode: 'pac', url: '', pacUrl: 'https://wpad/p.pac', noProxy: ['intranet'] },
      ai: { enabled: null, allowCloud: false },
      transfers: { maxUploadKbps: 512, maxDownloadKbps: null, maxConcurrentRuns: 2 },
      mounts: { allowedProtocols: ['s3', 'rclone'], readOnly: true, cacheMode: 'full', maxCacheMb: 2048, maxCacheAgeHours: null, mountRoot: '~/Drives' },
      workflows: [
        { id: 'tidy-downloads', name: 'Tidy Downloads!', enabled: true, rule: RULE },
        { id: 'archive', name: 'Archive', enabled: false, rule: RULE },
      ],
      backupTemplates: [{ id: 'org-standard', name: 'Org Standard', description: '', enforced: true, settings: { retention: { type: 'keep_count', count: 7 } } }],
      notificationProfiles: [{ id: 'on-call-pager', name: 'On-call pager', description: '', minSeverity: 'error', notification: { type: 'webhook', preset: 'pagerduty', url: 'https://events.pagerduty.com/v2/enqueue', credential_id: 'central:pd', headers: [{ name: 'X-Key', secret: true, credential_id: 'central:pdh' }] } }],
      serverAccess: { users: [{ username: 'ops', passwordHash: '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g', role: 'operator', disabled: false }], rules: [{ principal: 'ops', jobPattern: 'prod-*', permission: 'run' }] },
    });
    expect(b.policySetVersion).not.toBe(v0);
    const hbv = await json<{ policiesChanged: boolean }>(await hb(m.token, { policySetVersion: v0 }));
    expect(hbv.body.policiesChanged).toBe(true);

    // Compliance follows what agents report as applied.
    await applied.POST(await req('POST', '/api/agent/policies/applied', { headers: bearer(m.token), body: { applied: { managed_settings: 1 } } }));
    const view = await json<{ upToDate: number; desktops: number }>(await managedSettings.GET(await req('GET', '/api/fleet/managed-settings', { as: auditor })));
    expect(view.body).toMatchObject({ upToDate: 1, desktops: 1 });

    // Stopping: agents get null, and the next change is a newer version.
    expect((await json<{ settings: null }>(await put(admin, { updates: null, proxy: null, ai: { enabled: null, allowCloud: null } }))).body.settings).toBeNull();
    expect((await bundle()).managedSettings).toBeNull();
    expect((await json<{ settings: { version: number } }>(await put(admin, { ai: { enabled: false } }))).body.settings.version).toBe(3);
    expect(listAudit({}).entries.filter((e) => e.eventType === 'fleet.managed_settings')).toHaveLength(3);
  });
});
