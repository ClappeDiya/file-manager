import { describe, expect, it } from 'vitest';
import Database from 'better-sqlite3';
import { hash } from 'bcryptjs';
import { db, migrate, schemaVersion } from '@/lib/db';
import { LATEST_SCHEMA } from '@/lib/db/migrations';
import { appendAudit, anchorAudit, verifyAudit } from '@/lib/db/repos/audit';
import { createOrg, effectiveRole, setMembership } from '@/lib/db/repos/orgs';
import { getUser, updateUser } from '@/lib/db/repos/users';
import { beginEnrollment, confirmEnrollment } from '@/lib/db/repos/mfa';
import { hotp, totpStep, verifyTotp, base32Decode, base32Encode } from '@/lib/totp';
import { open, seal } from '@/lib/crypto';
import { cookieFrom, json, makeUser, req } from './helpers';
import * as policies from '@/app/api/policies/route';
import * as approvals from '@/app/api/approvals/route';
import * as users from '@/app/api/users/route';
import * as login from '@/app/api/auth/login/route';
import * as mfaVerify from '@/app/api/auth/mfa/verify/route';
import * as session from '@/app/api/auth/session/route';
import * as auditRoute from '@/app/api/audit/route';
import * as auditVerify from '@/app/api/audit/verify/route';

describe('migrations', () => {
  it('creates the latest schema and the default organization', () => {
    expect(schemaVersion()).toBe(LATEST_SCHEMA);
    expect(db().prepare("SELECT name FROM organizations WHERE id = 'org_default'").get()).toBeTruthy();
  });

  it('upgrades a database written by the old unversioned store', () => {
    const d = new Database(':memory:');
    d.exec(`CREATE TABLE users (id TEXT PRIMARY KEY, email TEXT UNIQUE NOT NULL, name TEXT NOT NULL DEFAULT '', role TEXT NOT NULL DEFAULT 'user', password_hash TEXT, permissions TEXT DEFAULT '[]', organization_id TEXT DEFAULT '', last_login_at TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), is_active INTEGER NOT NULL DEFAULT 1, sso_provider TEXT, mfa_enabled INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE devices (id TEXT PRIMARY KEY, device_id TEXT UNIQUE NOT NULL, device_name TEXT NOT NULL DEFAULT '');
            CREATE TABLE audit_entries (id TEXT PRIMARY KEY, event_type TEXT NOT NULL, severity TEXT NOT NULL DEFAULT 'info', timestamp TEXT NOT NULL DEFAULT (datetime('now')), user_id TEXT DEFAULT '', user_email TEXT DEFAULT '', ip_address TEXT DEFAULT '', user_agent TEXT DEFAULT '', description TEXT NOT NULL DEFAULT '', metadata TEXT DEFAULT '{}', integrity_hash TEXT DEFAULT '', previous_hash TEXT DEFAULT '');
            INSERT INTO users (id, email, role, organization_id) VALUES ('u1', 'old@x.org', 'org_admin', ''), ('u2', 'acme@x.org', 'operator', 'org_acme');`);
    expect(migrate(d)).toBe(LATEST_SCHEMA);
    const cols = (d.prepare('PRAGMA table_info(devices)').all() as { name: string }[]).map((c) => c.name);
    expect(cols).toEqual(expect.arrayContaining(['user_name', 'os_version']));
    expect(d.prepare('SELECT org_id, role FROM org_memberships ORDER BY user_id').all()).toEqual([
      { org_id: 'org_default', role: 'org_admin' },
      { org_id: 'org_acme', role: 'operator' },
    ]);
    expect(migrate(d)).toBe(0);
  });
});

describe('roles and permissions', () => {
  it('maps database roles onto permissions (the old check let every role through)', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const auditor = makeUser('auditor@x.org', 'read_only_auditor');
    const plain = makeUser('plain@x.org', 'user');
    const create = { domain: 'retention', name: 'Keep 30 days', assignmentTarget: 'organization', assignmentValue: 'org_default' };
    expect((await policies.POST(await req('POST', '/api/policies', { as: auditor, body: create }))).status).toBe(403);
    expect((await users.GET(await req('GET', '/api/users', { as: plain }))).status).toBe(403);
    expect((await policies.GET(await req('GET', '/api/policies', { as: auditor }))).status).toBe(200);
    const r = await json<{ policy: { id: string } }>(await policies.POST(await req('POST', '/api/policies', { as: owner, body: create })));
    expect(r.status).toBe(201);
    expect((await policies.GET(await req('GET', '/api/policies', { as: auditor })).then((x) => x.json())).total).toBe(1);
  });

  it('applies role changes and deactivation immediately, despite an older cookie', async () => {
    const u = makeUser('ops@x.org', 'org_admin');
    const r1 = await req('GET', '/api/policies', { as: u });
    setMembership(u.id, 'org_default', 'user');
    expect((await policies.POST(await req('POST', '/api/policies', { as: u, body: { domain: 'retention', name: 'x', assignmentTarget: 'organization', assignmentValue: 'o' } }))).status).toBe(403);
    updateUser(u.id, { isActive: false });
    expect((await policies.GET(r1)).status).toBe(401);
  });
});

describe('governance CRUD (writes used to be lost)', () => {
  it('creates, updates, deletes policies and reviews approvals', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const approver = makeUser('appr@x.org', 'approver');
    const created = await json<{ policy: { id: string; version: number } }>(await policies.POST(await req('POST', '/api/policies', { as: owner, body: { domain: 'retention', name: 'P', assignmentTarget: 'organization', assignmentValue: 'org_default' } })));
    const id = created.body.policy.id;
    const upd = await json<{ policy: { name: string; version: number } }>(await policies.PUT(await req('PUT', '/api/policies', { as: owner, body: { id, name: 'P2', createdBy: 'evil' } })));
    expect(upd.body.policy).toMatchObject({ name: 'P2', version: 2 });
    expect((await policies.DELETE(await req('DELETE', `/api/policies?id=${id}`, { as: owner }))).status).toBe(200);
    expect((await policies.GET(await req('GET', '/api/policies', { as: owner })).then((x) => x.json())).total).toBe(0);

    const a = await json<{ approval: { id: string; requestedBy: string } }>(await approvals.POST(await req('POST', '/api/approvals', { as: owner, body: { operation: 'delete', trigger: 'destructive_sync', source: 's', destination: 'd', requestedBy: 'spoofed@x.org' } })));
    expect(a.body.approval.requestedBy).toBe('owner@x.org');
    expect((await approvals.PATCH(await req('PATCH', '/api/approvals', { as: owner, body: { id: a.body.approval.id, action: 'approve' } }))).status).toBe(403);
    const ok = await json<{ approval: { state: string } }>(await approvals.PATCH(await req('PATCH', '/api/approvals', { as: approver, body: { id: a.body.approval.id, action: 'approve' } })));
    expect(ok.body.approval.state).toBe('approved');
    expect((await approvals.PATCH(await req('PATCH', '/api/approvals', { as: approver, body: { id: a.body.approval.id, action: 'deny' } }))).status).toBe(400);
  });

  it('keeps organizations apart', async () => {
    const acme = createOrg({ name: 'Acme' });
    const a = makeUser('a@acme.org', 'org_owner', acme.id);
    const b = makeUser('b@x.org', 'org_owner');
    await policies.POST(await req('POST', '/api/policies', { as: a, body: { domain: 'retention', name: 'Acme only', assignmentTarget: 'organization', assignmentValue: acme.id } }));
    expect((await policies.GET(await req('GET', '/api/policies', { as: b })).then((x) => x.json())).total).toBe(0);
    // A forged org claim for an org without membership is refused.
    expect((await policies.GET(await req('GET', '/api/policies', { as: b, org: acme.id }))).status).toBe(403);
  });
});

describe('audit log', () => {
  it('is append-only and hash-chained; tampering is pinpointed', async () => {
    for (let i = 0; i < 5; i++) appendAudit({ eventType: 'test.event', description: `event ${i}`, orgId: 'org_default' });
    expect(verifyAudit()).toMatchObject({ ok: true, entriesChecked: 5 });
    expect(() => db().prepare("UPDATE audit_entries SET description = 'x' WHERE sequence = 2").run()).toThrow(/append-only/);
    expect(() => db().prepare('DELETE FROM audit_entries WHERE sequence = 5').run()).toThrow(/append-only/);
    anchorAudit();
    // Someone with raw file access drops the triggers and edits an entry.
    db().exec('DROP TRIGGER audit_no_update; DROP TRIGGER audit_no_delete;');
    db().prepare("UPDATE audit_entries SET description = 'forged' WHERE sequence = 3").run();
    expect(verifyAudit()).toMatchObject({ ok: false, firstBadSequence: 3 });
  });

  it('detects truncation after a signed anchor', () => {
    for (let i = 0; i < 4; i++) appendAudit({ eventType: 'test.event', description: `e${i}` });
    anchorAudit();
    db().exec('DROP TRIGGER audit_no_delete;');
    db().prepare('DELETE FROM audit_entries WHERE sequence = 4').run();
    const v = verifyAudit();
    expect(v.ok).toBe(false);
    expect(v.reason).toMatch(/removed|differs/);
  });

  it('records sign-ins and exposes verification through the API', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    await policies.POST(await req('POST', '/api/policies', { as: owner, body: { domain: 'retention', name: 'P', assignmentTarget: 'organization', assignmentValue: 'o' } }));
    const list = await json<{ entries: { eventType: string }[] }>(await auditRoute.GET(await req('GET', '/api/audit', { as: owner })));
    expect(list.body.entries.map((e) => e.eventType)).toContain('policy.create');
    const anchored = await json<{ anchor: { lastSequence: number }; verification: { ok: boolean } }>(await auditVerify.POST(await req('POST', '/api/audit/verify', { as: owner })));
    expect(anchored.body.verification.ok).toBe(true);
    const csv = await auditRoute.GET(await req('GET', '/api/audit?export=csv', { as: owner }));
    expect(await csv.text()).toContain('policy.create');
  });
});

describe('sign-in', () => {
  it('logs in with a password, rate-limits failures, and returns org-scoped session info', async () => {
    makeUser('pw@x.org', 'org_admin', 'org_default', { passwordHash: await hash('s3cret-pass', 4) });
    expect((await login.POST(await req('POST', '/api/auth/login', { body: { email: 'pw@x.org', password: 'nope' } }))).status).toBe(401);
    const ok = await login.POST(await req('POST', '/api/auth/login', { body: { email: 'PW@x.org', password: 's3cret-pass' } }));
    expect(ok.status).toBe(200);
    const cookie = cookieFrom(ok)!;
    const s = await json<{ role: string; organization: { id: string }; permissions: string[] }>(await session.GET(await req('GET', '/api/auth/session', { headers: { cookie: `ufop-admin-session=${cookie}` } })));
    expect(s.body).toMatchObject({ role: 'org_admin', organization: { id: 'org_default' } });
    expect(s.body.permissions).toContain('fleet.write');
    for (let i = 0; i < 5; i++) await login.POST(await req('POST', '/api/auth/login', { body: { email: 'pw@x.org', password: 'bad' } }));
    expect((await login.POST(await req('POST', '/api/auth/login', { body: { email: 'pw@x.org', password: 's3cret-pass' } }))).status).toBe(429);
  });

  it('requires the TOTP second factor, rejects replays and accepts a recovery code once', async () => {
    const u = makeUser('mfa@x.org', 'org_owner', 'org_default', { passwordHash: await hash('pw-123456', 4) });
    const secret = beginEnrollment(u);
    expect(confirmEnrollment(getUser(u.id)!, '000000').ok).toBe(false);
    const enrolled = confirmEnrollment(getUser(u.id)!, hotp(secret, totpStep()));
    if (!enrolled.ok) throw new Error(enrolled.error);
    const first = await json<{ mfaRequired: boolean; challengeId: string }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: 'mfa@x.org', password: 'pw-123456' } })));
    expect(first.body.mfaRequired).toBe(true);
    // The code used for enrollment cannot be replayed.
    expect((await mfaVerify.POST(await req('POST', '/api/auth/mfa/verify', { body: { challengeId: first.body.challengeId, code: hotp(secret, totpStep()) } }))).status).toBe(401);
    const next = hotp(secret, totpStep() + 1);
    const done = await mfaVerify.POST(await req('POST', '/api/auth/mfa/verify', { body: { challengeId: first.body.challengeId, code: next } }));
    expect(done.status).toBe(200);
    expect(cookieFrom(done)).toBeTruthy();
    const again = await json<{ challengeId: string }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: 'mfa@x.org', password: 'pw-123456' } })));
    const rc = enrolled.recoveryCodes[0];
    expect((await mfaVerify.POST(await req('POST', '/api/auth/mfa/verify', { body: { challengeId: again.body.challengeId, code: rc } }))).status).toBe(200);
    const third = await json<{ challengeId: string }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: 'mfa@x.org', password: 'pw-123456' } })));
    expect((await mfaVerify.POST(await req('POST', '/api/auth/mfa/verify', { body: { challengeId: third.body.challengeId, code: rc } }))).status).toBe(401);
  });
});

describe('primitives', () => {
  it('TOTP matches the RFC 6238 SHA-1 test vector', () => {
    const secret = base32Encode(Buffer.from('12345678901234567890'));
    expect(base32Decode(secret).toString()).toBe('12345678901234567890');
    // RFC 6238 appendix B: T = 59 s → 94287082 (8 digits); 6-digit tail.
    expect(hotp(secret, Math.floor(59 / 30), 8)).toBe('94287082');
    expect(verifyTotp(secret, '287082', null, 59_000)).toBe(1);
  });

  it('seals secrets with AES-GCM and detects tampering', () => {
    const s = seal('top secret');
    expect(s).not.toContain('top secret');
    expect(open(s)).toBe('top secret');
    const parts = s.split('.');
    parts[3] = Buffer.from('tampered').toString('base64url');
    expect(() => open(parts.join('.'))).toThrow();
  });

  it('delegates roles downward but not into detached organizations', () => {
    const parent = createOrg({ name: 'Parent' });
    const child = createOrg({ name: 'Child', parentId: parent.id });
    const detached = createOrg({ name: 'Detached', parentId: parent.id, detached: true });
    const grand = createOrg({ name: 'Grand', parentId: detached.id });
    const u = makeUser('p@x.org', 'org_admin', parent.id);
    expect(effectiveRole(u.id, child.id)).toBe('org_admin');
    expect(effectiveRole(u.id, detached.id)).toBeNull();
    expect(effectiveRole(u.id, grand.id)).toBeNull();
  });
});
