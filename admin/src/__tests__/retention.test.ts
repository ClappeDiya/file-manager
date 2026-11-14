/** Log retention: settings, plain-log cleanup, and audit pruning only at a signed anchor (chain stays verifiable). */
import { describe, expect, it } from 'vitest';
import { db } from '@/lib/db';
import { anchorAudit, appendAudit, lastAuditPrune, pruneAudit, verifyAudit } from '@/lib/db/repos/audit';
import { createOrg } from '@/lib/db/repos/orgs';
import { createSiemDestination } from '@/lib/db/repos/siem';
import { applyRetention, getRetention, setRetention } from '@/lib/retention';
import { json, makeUser, req } from './helpers';
import * as retention from '@/app/api/retention/route';

const DAY = 86_400_000;
/** Timestamps are part of each entry's hash, so tests move the cutoff instead of backdating. */
const future = (days: number) => new Date(Date.now() + days * DAY);

describe('retention', () => {
  it('validates settings and only lets top-level owners change them', async () => {
    const top = createOrg({ name: 'Top org' });
    const child = createOrg({ name: 'Child org', parentId: top.id });
    const owner = makeUser('ret-owner@x.org', 'org_owner', top.id);
    const childOwner = makeUser('ret-child@x.org', 'org_owner', child.id);
    expect(getRetention()).toMatchObject({ audit: 0, api_requests: 30 });
    expect(() => setRetention({ audit: 30 }, 't')).toThrow(/365 to 3650/);
    expect(() => setRetention({ api_requests: 0 }, 't')).toThrow(/7 to 365/);
    expect(() => setRetention({ nope: 5 }, 't')).toThrow(/Unknown log type/);
    const put = async (as: typeof owner, body: Record<string, unknown>) => json<{ settings?: Record<string, number>; error?: string }>(await retention.PUT(await req('PUT', '/api/retention', { as, body })));
    expect((await put(childOwner, { api_requests: 14 })).status).toBe(403);
    const r = await put(owner, { api_requests: 14 });
    expect(r.status).toBe(200);
    expect(r.body.settings!.api_requests).toBe(14);
    const view = await json<{ canEdit: boolean; auditVerifies: boolean }>(await retention.GET(await req('GET', '/api/retention', { as: childOwner })));
    expect(view.body).toMatchObject({ canEdit: false, auditVerifies: true });
  });

  it('prunes the audit log only up to a signed anchor, keeps it verifiable and still detects tampering', () => {
    const org = createOrg({ name: 'Prune org' });
    const start = Number((db().prepare('SELECT MAX(sequence) AS s FROM audit_entries').get() as { s: number | null }).s ?? 0);
    for (let i = 0; i < 10; i++) appendAudit({ eventType: 'old.event', orgId: org.id, description: `old ${i}` });
    const a1 = anchorAudit()!; // covers everything so far
    for (let i = 0; i < 5; i++) appendAudit({ eventType: 'old.event', orgId: org.id, description: `older but after anchor ${i}` });
    for (let i = 0; i < 3; i++) appendAudit({ eventType: 'new.event', orgId: org.id, description: `new ${i}` });
    const head = start + 18;

    // Nothing older than the cutoff -> nothing removed.
    expect(pruneAudit(new Date(Date.now() - 365 * DAY))).toBe(0);
    // Everything is older than a future cutoff, but only up to the newest anchor goes.
    expect(pruneAudit(future(1))).toBe(a1.lastSequence);
    expect(lastAuditPrune()).toMatchObject({ throughSequence: a1.lastSequence, throughHash: a1.lastHash });
    expect((db().prepare('SELECT MIN(sequence) AS s FROM audit_entries').get() as { s: number }).s).toBe(a1.lastSequence + 1);
    const v = verifyAudit();
    expect(v).toMatchObject({ ok: true, headSequence: head });
    // Nothing more until a newer anchor exists.
    expect(pruneAudit(future(1))).toBe(0);
    // Deleting is still blocked outside pruning.
    expect(() => db().prepare('DELETE FROM audit_entries WHERE sequence = ?').run(head)).toThrow(/append-only/);

    // Tampering after a prune is still detected, and then nothing is pruned.
    db().exec('DROP TRIGGER audit_no_update');
    db()
      .prepare("UPDATE audit_entries SET description = 'forged' WHERE sequence = ?")
      .run(a1.lastSequence + 2);
    db().exec("CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_entries BEGIN SELECT RAISE(ABORT, 'audit entries are append-only'); END");
    expect(verifyAudit()).toMatchObject({ ok: false, firstBadSequence: a1.lastSequence + 2 });
    anchorAudit();
    expect(() => pruneAudit(new Date())).toThrow(/does not verify/);
  });
});

describe('retention: applying', () => {
  it('cleans plain logs by age and never prunes audit entries SIEM has not forwarded', () => {
    const org = createOrg({ name: 'Apply org' });
    for (let i = 0; i < 4; i++) appendAudit({ eventType: 'e', orgId: org.id, description: `e${i}` });
    const a = anchorAudit()!;
    const siem = createSiemDestination(org.id, { name: 'slow', kind: 'https', config: { url: 'https://siem.example/x' }, backfill: true }, 't');
    db()
      .prepare('UPDATE siem_destinations SET cursor = ? WHERE id = ?')
      .run(a.lastSequence - 2, siem.id);
    // Old API log and SSO records.
    db()
      .prepare("INSERT INTO api_requests (id, org_id, at, method, path, status, outcome, auth) VALUES ('r1', ?, ?, 'GET', '/x', 200, 'authorized', 'token'), ('r2', ?, ?, 'GET', '/x', 200, 'authorized', 'token')")
      .run(org.id, new Date(Date.now() - 40 * DAY).toISOString(), org.id, new Date().toISOString());
    setRetention({ audit: 365, api_requests: 30 }, 't');
    const removed = applyRetention();
    expect(removed).toEqual({ api_requests: 1 }); // nothing in the audit log is a year old
    // A year and more later: the SIEM destination is behind the only anchor, so the audit log is kept.
    expect(applyRetention(future(400)).audit).toBeUndefined();
    db().prepare('UPDATE siem_destinations SET cursor = ? WHERE id = ?').run(a.lastSequence, siem.id);
    expect(applyRetention(future(400)).audit).toBe(a.lastSequence);
    expect(verifyAudit().ok).toBe(true);
  });
});
