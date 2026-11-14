/** Background exports: everything exported (no truncation), page boundaries, formats, ownership, expiry; the sync export refuses instead of truncating. */
import { describe, expect, it } from 'vitest';
import zlib from 'zlib';
import crypto from 'crypto';
import { db } from '@/lib/db';
import { appendAudit } from '@/lib/db/repos/audit';
import { createOrg } from '@/lib/db/repos/orgs';
import { processExports } from '@/lib/exports';
import { json, makeUser, req } from './helpers';
import * as exportsRoute from '@/app/api/exports/route';
import * as exportById from '@/app/api/exports/[id]/route';
import * as download from '@/app/api/exports/[id]/download/route';
import * as auditRoute from '@/app/api/audit/route';

type U = ReturnType<typeof makeUser>;
const P = (id: string) => ({ params: Promise.resolve({ id }) });
const start = async (as: U, body: unknown) => json<{ export: { id: string; status: string }; error?: string }>(await exportsRoute.POST(await req('POST', '/api/exports', { as, body })));
const get = async (as: U, id: string) => json<{ export: { status: string; total: number; done: number; size: number; sha256: string } }>(await exportById.GET(await req('GET', `/api/exports/${id}`, { as }), P(id)));

describe('background exports', () => {
  it('exports every matching audit entry across pages, oldest first, in each format', async () => {
    const org = createOrg({ name: 'Export org' });
    const admin = makeUser('exp-admin@x.org', 'org_admin', org.id);
    const n = 2500;
    for (let i = 0; i < n; i++) appendAudit({ eventType: i % 2 ? 'odd.event' : 'even.event', orgId: org.id, description: `entry ${i}` });
    appendAudit({ eventType: 'other.org', orgId: createOrg({ name: 'Elsewhere' }).id, description: 'not mine' });

    // The export's own audit event ('export.queue') may fall inside its snapshot, so filter to the seeded entries.
    const csv = await start(admin, { kind: 'audit', format: 'csv', params: { search: 'entry ' } });
    const jsonExp = await start(admin, { kind: 'audit', format: 'json', params: { search: 'entry 1' } });
    const syslog = await start(admin, { kind: 'audit', format: 'syslog', params: { search: 'entry ' } });
    expect(csv.status).toBe(202);
    await processExports();

    const c = await get(admin, csv.body.export.id);
    expect(c.body.export).toMatchObject({ status: 'succeeded', total: n, done: n });
    const res = await download.GET(await req('GET', `/api/exports/${csv.body.export.id}/download`, { as: admin }), P(csv.body.export.id));
    const text = await res.text();
    expect(res.headers.get('content-disposition')).toMatch(/audit-\d{4}-\d{2}-\d{2}\.csv/);
    const lines = text.trim().split('\n');
    expect(lines).toHaveLength(n + 1);
    expect(lines[0]).toMatch(/^ID,Event Type/);
    expect(lines.filter((l) => l.startsWith('ID,'))).toHaveLength(1); // one header across pages
    expect(lines[1]).toContain('entry 0');
    expect(lines[n]).toContain(`entry ${n - 1}`);
    expect(text).not.toContain('not mine');
    expect(crypto.createHash('sha256').update(text).digest('hex')).toBe(c.body.export.sha256);
    // The gzip download is the stored file.
    const gz = Buffer.from(await (await download.GET(await req('GET', `/api/exports/${csv.body.export.id}/download?gz=1`, { as: admin }), P(csv.body.export.id))).arrayBuffer());
    expect(zlib.gunzipSync(gz).toString()).toBe(text);

    const j = JSON.parse(await (await download.GET(await req('GET', `/api/exports/${jsonExp.body.export.id}/download`, { as: admin }), P(jsonExp.body.export.id))).text()) as { description: string; sequence: number }[];
    const expected = Array.from({ length: n }, (_, i) => `entry ${i}`).filter((d) => d.includes('entry 1'));
    expect(j.map((e) => e.description)).toEqual(expected);
    expect(j.map((e) => e.sequence)).toEqual([...j.map((e) => e.sequence)].sort((a, b) => a - b));
    const s = await (await download.GET(await req('GET', `/api/exports/${syslog.body.export.id}/download`, { as: admin }), P(syslog.body.export.id))).text();
    expect(s.trim().split('\n')).toHaveLength(n);
    expect(s.split('\n')[0]).toMatch(/^<6>1 /);
  });

  it('checks formats, permissions and ownership, and expires files', async () => {
    const org = createOrg({ name: 'Export rules' });
    const admin = makeUser('exp-a@x.org', 'org_admin', org.id);
    const approver = makeUser('exp-b@x.org', 'approver', org.id);
    expect((await start(admin, { kind: 'machines', format: 'syslog' })).status).toBe(400);
    expect((await start(admin, { kind: 'nope', format: 'csv' })).status).toBe(400);
    expect((await start(approver, { kind: 'audit', format: 'csv' })).status).toBe(403); // needs audit.export
    const m = await start(admin, { kind: 'machines', format: 'csv' });
    await processExports();
    const csv = await (await download.GET(await req('GET', `/api/exports/${m.body.export.id}/download`, { as: admin }), P(m.body.export.id))).text();
    expect(csv.split('\n')[0]).toBe('id,name,hostname,os,osVersion,clientVersion,status,connectivity,lastSeenAt,lastIp,tags,registeredAt');
    // Someone else in the organization cannot see or download it.
    const other = makeUser('exp-c@x.org', 'org_admin', org.id);
    expect((await get(other, m.body.export.id)).status).toBe(404);
    expect((await download.GET(await req('GET', `/api/exports/${m.body.export.id}/download`, { as: other }), P(m.body.export.id))).status).toBe(404);
    // Expiry drops the file.
    db()
      .prepare('UPDATE export_jobs SET expires_at = ? WHERE id = ?')
      .run(new Date(Date.now() - 1000).toISOString(), m.body.export.id);
    await processExports();
    expect((await get(admin, m.body.export.id)).body.export.status).toBe('expired');
    const gone = await json<{ error: string }>(await download.GET(await req('GET', `/api/exports/${m.body.export.id}/download`, { as: admin }), P(m.body.export.id)));
    expect(gone.status).toBe(409);
    expect(gone.body.error).toMatch(/expired/);
    expect((await exportById.DELETE(await req('DELETE', `/api/exports/${m.body.export.id}`, { as: admin }), P(m.body.export.id))).status).toBe(200);
  });

  it('the immediate audit export refuses more than 10,000 entries instead of cutting them off (regression)', async () => {
    const org = createOrg({ name: 'Big audit' });
    const admin = makeUser('big@x.org', 'org_admin', org.id);
    for (let i = 0; i < 10_001; i++) appendAudit({ eventType: 'bulk', orgId: org.id, description: `b${i}` });
    const r = await json<{ code: string; total: number }>(await auditRoute.GET(await req('GET', '/api/audit?export=csv', { as: admin })));
    expect(r.status).toBe(413);
    expect(r.body).toMatchObject({ code: 'use_background_export' });
    expect(r.body.total).toBeGreaterThan(10_000);
    const small = await auditRoute.GET(await req('GET', '/api/audit?export=csv&search=b10000', { as: admin }));
    expect(small.status).toBe(200);
  }, 60_000);
});
