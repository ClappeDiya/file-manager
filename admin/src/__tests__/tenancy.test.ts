/**
 * Tenancy end to end through the route handlers: organization tree and
 * delegation, detached subtrees, isolation, switching, licenses, offboarding,
 * groups (roles, members, grants), access scoping of the `user` role,
 * invitations and bulk user actions.
 */
import { describe, expect, it } from 'vitest';
import { db } from '@/lib/db';
import { createOrg, effectiveRole, getOrg, membersOf, purgeOffboarded, setMembership } from '@/lib/db/repos/orgs';
import { createLink, getMachine, registerWithToken } from '@/lib/db/repos/machines';
import { ingestReport, normalizeReport } from '@/lib/db/repos/reports';
import { createGroup, setGroupMember } from '@/lib/db/repos/groups';
import { createRule } from '@/lib/db/repos/alerts';
import { evaluateReport } from '@/lib/alerts';
import { getUser } from '@/lib/db/repos/users';
import { listAudit, verifyAudit } from '@/lib/db/repos/audit';
import { cookieFrom, json, makeUser, req } from './helpers';
import * as orgs from '@/app/api/orgs/route';
import * as orgById from '@/app/api/orgs/[id]/route';
import * as orgSwitch from '@/app/api/orgs/switch/route';
import * as groups from '@/app/api/groups/route';
import * as groupById from '@/app/api/groups/[id]/route';
import * as groupMembers from '@/app/api/groups/[id]/members/route';
import * as groupGrants from '@/app/api/groups/[id]/grants/route';
import * as invitationById from '@/app/api/invitations/[id]/route';
import * as accept from '@/app/api/invitations/accept/route';
import * as users from '@/app/api/users/route';
import * as machines from '@/app/api/fleet/machines/route';
import * as machineById from '@/app/api/fleet/machines/[id]/route';
import * as commands from '@/app/api/fleet/machines/[id]/commands/route';
import * as reports from '@/app/api/fleet/reports/route';
import * as dashboard from '@/app/api/fleet/dashboard/route';
import * as alerts from '@/app/api/alerts/route';
import * as fleetPolicies from '@/app/api/fleet/policies/route';
import * as heartbeat from '@/app/api/agent/heartbeat/route';
import * as register from '@/app/api/agent/register/route';

const P = (id: string) => ({ params: Promise.resolve({ id }) });
const cookie = (c: string) => ({ cookie: `ufop-admin-session=${c}` });

function enroll(orgId: string, name: string, tags: string[] = []) {
  const { token } = createLink(orgId, { label: 'test' }, 'test');
  return registerWithToken(token, { name, hostname: name, os: 'linux' }, tags);
}

function report(orgId: string, machineId: string, backupId: string, status = 'success') {
  return ingestReport(orgId, machineId, normalizeReport({ backup_id: backupId, name: backupId, result: status }), {});
}

describe('organization tree', () => {
  it('creates child organizations and delegates roles downward', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const op = makeUser('op@x.org', 'operator');
    const created = await json<{ organization: { id: string; parentId: string } }>(await orgs.POST(await req('POST', '/api/orgs', { as: owner, body: { name: 'Branch' } })));
    expect(created.status).toBe(201);
    const child = created.body.organization.id;
    expect(created.body.organization.parentId).toBe('org_default');
    // Operators can read but not create organizations.
    expect((await orgs.POST(await req('POST', '/api/orgs', { as: op, body: { name: 'Nope' } }))).status).toBe(403);
    // Delegation: both see the child with their parent role.
    const list = await json<{ organizations: { id: string; role: string; path: string }[] }>(await orgs.GET(await req('GET', '/api/orgs', { as: op })));
    expect(list.body.organizations.find((o) => o.id === child)).toMatchObject({ role: 'operator', path: 'Default organization / Branch' });
    const detail = await json<{ children: { id: string }[] }>(await orgById.GET(await req('GET', '/api/orgs/org_default', { as: owner }), P('org_default')));
    expect(detail.body.children.map((c) => c.id)).toEqual([child]);
    // A child admin cannot create organizations next to its own (in the parent).
    const childAdmin = makeUser('ca@x.org', 'org_admin', child);
    expect((await orgs.POST(await req('POST', '/api/orgs', { as: childAdmin, body: { name: 'Sibling', parentId: 'org_default' } }))).status).toBe(403);
    expect((await orgs.POST(await req('POST', '/api/orgs', { as: childAdmin, body: { name: 'Grandchild' } }))).status).toBe(201);
    // Moving an organization under its own descendant is refused.
    expect((await orgById.PATCH(await req('PATCH', `/api/orgs/${child}`, { as: owner, body: { parentId: child } }), P(child))).status).toBe(400);
  });

  it('does not let the parent manage a detached subtree', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const admin = makeUser('admin@x.org', 'org_admin');
    const r = await json<{ organization: { id: string; detached: boolean } }>(await orgs.POST(await req('POST', '/api/orgs', { as: owner, body: { name: 'Spin-off', detached: true } })));
    const det = r.body.organization.id;
    expect(r.body.organization.detached).toBe(true);
    // The creator owns it directly; other parent admins have no access at all.
    expect(effectiveRole(owner.id, det)).toBe('org_owner');
    expect(effectiveRole(admin.id, det)).toBeNull();
    expect((await orgById.GET(await req('GET', `/api/orgs/${det}`, { as: admin }), P(det))).status).toBe(404);
    expect((await orgById.PATCH(await req('PATCH', `/api/orgs/${det}`, { as: admin, body: { name: 'Taken over' } }), P(det))).status).toBe(403);
    expect((await orgSwitch.POST(await req('POST', '/api/orgs/switch', { as: admin, body: { orgId: det } }))).status).toBe(403);
    // Detaching and protection are owner-only.
    const child = createOrg({ name: 'Attached', parentId: 'org_default' });
    expect((await orgById.PATCH(await req('PATCH', `/api/orgs/${child.id}`, { as: admin, body: { detached: true } }), P(child.id))).status).toBe(403);
    expect((await orgById.PATCH(await req('PATCH', `/api/orgs/${child.id}`, { as: owner, body: { detached: true } }), P(child.id))).status).toBe(200);
    expect(effectiveRole(admin.id, child.id)).toBeNull();
  });

  it('keeps organizations isolated', async () => {
    const a = createOrg({ name: 'A' });
    const b = createOrg({ name: 'B' });
    const ua = makeUser('a@a.org', 'org_owner', a.id);
    makeUser('b@b.org', 'org_owner', b.id);
    const { machine } = enroll(b.id, 'b-server');
    const rpt = report(b.id, machine.id, 'job', 'error');
    createRule(b.id, { name: 'fail', kind: 'backup_failed' });
    await evaluateReport(getMachine(machine.id)!, rpt);
    await fleetPolicies.POST(await req('POST', '/api/fleet/policies', { as: makeUser('b2@b.org', 'org_admin', b.id), body: { name: 'B policy', target: { all: true }, job: { name: 'j', sources: ['/'], destination: { type: 'local', path: '/x' }, encrypted: false } } }));

    const ms = await json<{ machines: unknown[] }>(await machines.GET(await req('GET', '/api/fleet/machines', { as: ua })));
    expect(ms.body.machines).toEqual([]);
    expect((await machineById.GET(await req('GET', `/api/fleet/machines/${machine.id}`, { as: ua }), P(machine.id))).status).toBe(404);
    expect((await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${machine.id}`, { as: ua, body: { action: 'revoke' } }), P(machine.id))).status).toBe(404);
    expect((await commands.POST(await req('POST', `/api/fleet/machines/${machine.id}/commands`, { as: ua, body: { kind: 'apply_policies' } }), P(machine.id))).status).toBe(404);
    expect((await reports.GET(await req('GET', `/api/fleet/reports?id=${rpt.id}`, { as: ua }))).status).toBe(404);
    expect((await json<{ reports: unknown[] }>(await reports.GET(await req('GET', '/api/fleet/reports', { as: ua })))).body.reports).toEqual([]);
    expect((await json<{ alerts: unknown[] }>(await alerts.GET(await req('GET', '/api/alerts?state=all', { as: ua })))).body.alerts).toEqual([]);
    expect((await json<{ policies: unknown[] }>(await fleetPolicies.GET(await req('GET', '/api/fleet/policies', { as: ua })))).body.policies).toEqual([]);
    // Moving a foreign machine into A is impossible as well.
    expect(getMachine(machine.id)!.orgId).toBe(b.id);
    // A forged org claim is refused.
    expect((await machines.GET(await req('GET', '/api/fleet/machines', { as: ua, org: b.id }))).status).toBe(403);
  });

  it('re-scopes the session when switching organizations', async () => {
    const a = createOrg({ name: 'A' });
    const b = createOrg({ name: 'B' });
    const u = makeUser('multi@x.org', 'org_admin', a.id);
    setMembership(u.id, b.id, 'read_only_auditor');
    enroll(b.id, 'only-in-b');
    expect((await json<{ machines: unknown[] }>(await machines.GET(await req('GET', '/api/fleet/machines', { as: u })))).body.machines).toHaveLength(0);
    const sw = await orgSwitch.POST(await req('POST', '/api/orgs/switch', { as: u, body: { orgId: b.id } }));
    expect(await sw.clone().json()).toEqual({ orgId: b.id, role: 'read_only_auditor' });
    const c = cookieFrom(sw)!;
    const inB = await json<{ machines: { name: string }[] }>(await machines.GET(await req('GET', '/api/fleet/machines', { headers: cookie(c) })));
    expect(inB.body.machines.map((m) => m.name)).toEqual(['only-in-b']);
    // The role follows the organization: an auditor cannot write in B.
    expect((await fleetPolicies.POST(await req('POST', '/api/fleet/policies', { headers: cookie(c), body: {} }))).status).toBe(403);
    expect((await orgSwitch.POST(await req('POST', '/api/orgs/switch', { as: u, body: { orgId: 'org_default' } }))).status).toBe(403);
    expect(listAudit({ orgIds: [b.id], eventType: 'org.switch' }).total).toBe(1);
  });
});

describe('licenses', () => {
  it('enforces seats, inherits them downward but not into detached organizations', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const parent = (await json<{ organization: { id: string } }>(await orgs.POST(await req('POST', '/api/orgs', { as: owner, body: { name: 'Reseller', licensePlan: 'pro', licenseSeats: 2 } })))).body.organization.id;
    const child = createOrg({ name: 'Customer', parentId: parent });
    const det = createOrg({ name: 'Independent', parentId: parent, detached: true });
    const childAdmin = makeUser('ca@x.org', 'org_admin', child.id);
    // A license is granted from above: the child's own admins cannot raise it.
    expect((await orgById.PATCH(await req('PATCH', `/api/orgs/${child.id}`, { as: childAdmin, body: { licenseSeats: 100 } }), P(child.id))).status).toBe(403);

    const { token } = createLink(child.id, {}, 't');
    const reg = async (n: string) => register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: token, info: { name: n } } }));
    expect((await reg('one')).status).toBe(201);
    enroll(parent, 'two');
    const third = await json(await reg('three'));
    expect(third.status).toBe(402);
    expect(third.body.error).toMatch(/2 machine/);
    const lic = await json<{ organization: { license: { seats: number; used: number; inherited: boolean; sourceOrgId: string } } }>(await orgById.GET(await req('GET', `/api/orgs/${child.id}`, { as: owner }), P(child.id)));
    expect(lic.body.organization.license).toMatchObject({ seats: 2, used: 2, inherited: true, sourceOrgId: parent });
    // The detached tenant has its own (unlimited) license.
    expect(enroll(det.id, 'free').machine.orgId).toBe(det.id);
    // The parent's manager raises the child's license.
    expect((await orgById.PATCH(await req('PATCH', `/api/orgs/${child.id}`, { as: owner, body: { licenseSeats: 5 } }), P(child.id))).status).toBe(200);
    expect((await reg('three')).status).toBe(201);
  });
});

describe('offboarding', () => {
  it('revokes access now, can be cancelled, and purges after the grace period', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const admin = makeUser('admin@x.org', 'org_admin');
    const org = createOrg({ name: 'Leaving', parentId: 'org_default' });
    const sub = createOrg({ name: 'Leaving sub', parentId: org.id });
    const { machine, token } = enroll(sub.id, 'srv');
    report(sub.id, machine.id, 'job');
    const del = async (as: typeof owner, body: unknown, id = org.id) => orgById.DELETE(await req('DELETE', `/api/orgs/${id}`, { as, body }), P(id));

    expect((await del(admin, { confirmName: 'Leaving' })).status).toBe(403);
    expect((await del(owner, { confirmName: 'leaving' })).status).toBe(400);
    expect((await del(owner, { confirmName: 'Default organization' }, 'org_default')).status).toBe(409);
    // A protected sub-organization blocks the delete.
    db().prepare('UPDATE organizations SET protected = 1 WHERE id = ?').run(sub.id);
    expect((await del(owner, { confirmName: 'Leaving' })).status).toBe(409);
    db().prepare('UPDATE organizations SET protected = 0 WHERE id = ?').run(sub.id);

    const started = await json<{ organization: { status: string; offboardAfter: string } }>(await del(owner, { confirmName: 'Leaving', graceDays: 10 }));
    expect(started.status).toBe(200);
    expect(started.body.organization.status).toBe('offboarding');
    expect(getOrg(sub.id)!.status).toBe('offboarding');
    // Machine access is revoked immediately.
    expect((await heartbeat.POST(await req('POST', '/api/agent/heartbeat', { body: {}, headers: { authorization: `Bearer ${token}` } }))).status).toBe(401);
    expect(getMachine(machine.id)!.status).toBe('revoked');

    const cancelled = await json<{ organization: { status: string } }>(await orgById.PATCH(await req('PATCH', `/api/orgs/${org.id}`, { as: owner, body: { action: 'cancel_offboarding' } }), P(org.id)));
    expect(cancelled.body.organization.status).toBe('active');
    expect(getOrg(sub.id)!.status).toBe('active');

    await del(owner, { confirmName: 'Leaving', graceDays: 1 });
    expect(purgeOffboarded(new Date())).toEqual([]);
    expect(purgeOffboarded(new Date(Date.now() + 2 * 86_400_000)).sort()).toEqual([org.id, sub.id].sort());
    expect(getOrg(org.id)).toBeNull();
    expect(db().prepare('SELECT COUNT(*) n FROM backup_reports WHERE org_id = ?').get(sub.id)).toEqual({ n: 0 });
    const events = listAudit({ eventType: undefined }).entries.map((e) => e.eventType);
    expect(events).toEqual(expect.arrayContaining(['org.offboard', 'org.offboard_cancel']));
  });
});

describe('groups', () => {
  it('grant organization roles, delegated to children only on request', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const admin = makeUser('admin@x.org', 'org_admin');
    const child = createOrg({ name: 'Child', parentId: 'org_default' });
    const u = makeUser('u@x.org', 'user');
    // An admin cannot grant a role at or above their own through a group.
    expect((await groups.POST(await req('POST', '/api/groups', { as: admin, body: { name: 'Owners', orgRole: 'org_owner' } }))).status).toBe(403);
    const g = (await json<{ group: { id: string } }>(await groups.POST(await req('POST', '/api/groups', { as: admin, body: { name: 'Ops', orgRole: 'operator' } })))).body.group.id;
    expect((await groupMembers.POST(await req('POST', `/api/groups/${g}/members`, { as: admin, body: { userId: u.id } }), P(g))).status).toBe(200);
    expect(effectiveRole(u.id, 'org_default')).toBe('operator');
    // Only the direct membership delegates downward.
    expect(effectiveRole(u.id, child.id)).toBe('user');
    // Delegation to sub-organizations.
    expect((await groupById.PATCH(await req('PATCH', `/api/groups/${g}`, { as: admin, body: { delegateToChildren: true } }), P(g))).status).toBe(200);
    expect(effectiveRole(u.id, child.id)).toBe('operator');
    // Deleting the group takes the role away.
    expect((await groupById.DELETE(await req('DELETE', `/api/groups/${g}`, { as: owner }), P(g))).status).toBe(200);
    expect(effectiveRole(u.id, 'org_default')).toBe('user');
  });

  it('lets group owners manage members (not grants) and keeps one owner', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const lead = makeUser('lead@x.org', 'user');
    const peer = makeUser('peer@x.org', 'user');
    const g = createGroup('org_default', { name: 'Finance' });
    setGroupMember(g.id, lead.id, 'owner');
    expect((await groupMembers.POST(await req('POST', `/api/groups/${g.id}/members`, { as: peer, body: { userId: peer.id } }), P(g.id))).status).toBe(403);
    expect((await groupMembers.POST(await req('POST', `/api/groups/${g.id}/members`, { as: lead, body: { userId: peer.id } }), P(g.id))).status).toBe(200);
    expect((await groupGrants.PUT(await req('PUT', `/api/groups/${g.id}/grants`, { as: lead, body: { grants: [{ resourceType: 'tag', resourceId: 'x' }] } }), P(g.id))).status).toBe(403);
    expect((await groupMembers.PATCH(await req('PATCH', `/api/groups/${g.id}/members`, { as: lead, body: { userId: lead.id, role: 'member' } }), P(g.id))).status).toBe(409);
    expect((await groupMembers.DELETE(await req('DELETE', `/api/groups/${g.id}/members?userId=${lead.id}`, { as: admin }), P(g.id))).status).toBe(409);
    // Members may leave on their own.
    expect((await groupMembers.DELETE(await req('DELETE', `/api/groups/${g.id}/members?userId=${peer.id}`, { as: peer }), P(g.id))).status).toBe(200);
    // Unknown people get an invitation that adds them to the group.
    const inv = await json<{ invitation: { email: string; groupId: string; role: string }; link: string }>(await groupMembers.POST(await req('POST', `/api/groups/${g.id}/members`, { as: lead, body: { email: 'New@Corp.example' } }), P(g.id)));
    expect(inv.status).toBe(201);
    expect(inv.body.invitation).toMatchObject({ email: 'new@corp.example', groupId: g.id, role: 'user' });
    const token = decodeURIComponent(inv.body.link.split('/invite/')[1]);
    const joined = await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, name: 'New', password: 'a-long-password-1' } }));
    expect(joined.status).toBe(200);
    const list = await json<{ groups: { id: string; members: { userId: string; role: string }[] }[] }>(await groups.GET(await req('GET', '/api/groups', { as: admin })));
    expect(list.body.groups[0].members.map((m) => m.role).sort()).toEqual(['member', 'owner']);
  });
});

describe('access scoping of the user role', () => {
  it('shows only machines granted directly, by backup or by tag', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const u = makeUser('owner-of-laptop@x.org', 'user');
    const m1 = enroll('org_default', 'finance-1', ['finance']).machine;
    const m2 = enroll('org_default', 'laptop').machine;
    const m3 = enroll('org_default', 'nas').machine;
    const m4 = enroll('org_default', 'hidden').machine;
    for (const m of [m1, m2, m3, m4]) report('org_default', m.id, 'job', 'error');
    createRule('org_default', { name: 'fail', kind: 'backup_failed' });
    for (const m of [m1, m2, m3, m4]) await evaluateReport(getMachine(m.id)!, report('org_default', m.id, 'job', 'error'));
    const g = createGroup('org_default', { name: 'Owners' });
    setGroupMember(g.id, u.id, 'member');
    const put = await groupGrants.PUT(await req('PUT', `/api/groups/${g.id}/grants`, { as: admin, body: { grants: [{ resourceType: 'tag', resourceId: 'finance' }, { resourceType: 'machine', resourceId: m2.id }, { resourceType: 'backup', resourceId: `${m3.id}/job` }, { resourceType: 'bogus', resourceId: 'x' }] } }), P(g.id));
    expect((await json<{ grants: unknown[] }>(put)).body.grants).toHaveLength(3);

    const seen = await json<{ machines: { id: string }[] }>(await machines.GET(await req('GET', '/api/fleet/machines', { as: u })));
    expect(seen.body.machines.map((m) => m.id).sort()).toEqual([m1.id, m2.id, m3.id].sort());
    expect((await machineById.GET(await req('GET', `/api/fleet/machines/${m4.id}`, { as: u }), P(m4.id))).status).toBe(404);
    expect((await machineById.GET(await req('GET', `/api/fleet/machines/${m2.id}`, { as: u }), P(m2.id))).status).toBe(200);
    const rs = await json<{ reports: { machineId: string }[] }>(await reports.GET(await req('GET', '/api/fleet/reports', { as: u })));
    expect(new Set(rs.body.reports.map((r) => r.machineId)).has(m4.id)).toBe(false);
    const dash = await json<{ machines: { total: number }; openAlerts: { machineId: string }[] }>(await dashboard.GET(await req('GET', '/api/fleet/dashboard?includeChildren=1', { as: u })));
    expect(dash.body.machines.total).toBe(3);
    expect(dash.body.openAlerts.map((a) => a.machineId)).not.toContain(m4.id);
    // Machine owners have no write or remote access.
    expect((await machineById.PATCH(await req('PATCH', `/api/fleet/machines/${m2.id}`, { as: u, body: { name: 'mine' } }), P(m2.id))).status).toBe(403);
    expect((await commands.POST(await req('POST', `/api/fleet/machines/${m2.id}/commands`, { as: u, body: { kind: 'apply_policies' } }), P(m2.id))).status).toBe(403);
    // Admins see everything.
    expect((await json<{ machines: unknown[] }>(await machines.GET(await req('GET', '/api/fleet/machines', { as: admin })))).body.machines).toHaveLength(4);
  });

  it('does not widen a non-delegated group role to sub-organizations on the dashboard', async () => {
    const child = createOrg({ name: 'Child', parentId: 'org_default' });
    const u = makeUser('grp@x.org', 'read_only_auditor', child.id);
    // u is an operator in the parent through a group that does not delegate.
    const g = createGroup('org_default', { name: 'Parent ops', orgRole: 'operator' });
    setGroupMember(g.id, u.id, 'member');
    enroll('org_default', 'parent-srv');
    enroll(child.id, 'child-srv');
    const grandchild = createOrg({ name: 'Grandchild', parentId: child.id });
    enroll(grandchild.id, 'gc-srv');
    expect(effectiveRole(u.id, grandchild.id)).toBe('read_only_auditor');
    const d = await json<{ machines: { total: number } }>(await dashboard.GET(await req('GET', '/api/fleet/dashboard?includeChildren=1', { as: u, org: 'org_default' })));
    // Child and grandchild are reachable only through u's own child membership, not the parent group:
    // the dashboard must not include organizations u has no fleet.read in.
    expect(effectiveRole(u.id, child.id)).toBe('read_only_auditor');
    expect(d.body.machines.total).toBe(3);
    // Without the child membership, the parent group alone gives no view into the child.
    db().prepare('DELETE FROM org_memberships WHERE user_id = ? AND org_id = ?').run(u.id, child.id);
    expect(effectiveRole(u.id, child.id)).toBeNull();
    const d2 = await json<{ machines: { total: number } }>(await dashboard.GET(await req('GET', '/api/fleet/dashboard?includeChildren=1', { as: u, org: 'org_default' })));
    expect(d2.body.machines.total).toBe(1);
  });
});

describe('invitations', () => {
  it('creates, accepts with a new account, and refuses reuse and expired links', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    // Not at or above one's own role.
    expect((await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'peer@x.org', role: 'org_admin' } }))).status).toBe(403);
    const r = await json<{ invitation: { id: string; status: string }; link: string; emailed: boolean }>(await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'Ops@Corp.example', role: 'operator' } })));
    expect(r.status).toBe(201);
    expect(r.body.emailed).toBe(false);
    const token = decodeURIComponent(r.body.link.split('/invite/')[1]);
    const info = await json(await accept.GET(await req('GET', `/api/invitations/accept?token=${encodeURIComponent(token)}`)));
    expect(info.body).toMatchObject({ email: 'ops@corp.example', role: 'operator', organization: 'Default organization', status: 'pending' });
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, password: 'short' } }))).status).toBe(400);
    const ok = await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, name: 'Ops', password: 'correct-horse-battery' } }));
    expect(ok.status).toBe(200);
    expect(cookieFrom(ok)).toBeTruthy();
    const roster = await json<{ users: { email: string; role: string }[]; invitations: { status: string }[] }>(await users.GET(await req('GET', '/api/users', { as: admin })));
    expect(roster.body.users.find((u) => u.email === 'ops@corp.example')?.role).toBe('operator');
    expect(roster.body.invitations[0].status).toBe('accepted');
    // Reuse is refused.
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, password: 'correct-horse-battery' } }))).status).toBe(410);
    // Expired.
    const e = await json<{ invitation: { id: string }; link: string }>(await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'late@corp.example' } })));
    db().prepare('UPDATE invitations SET expires_at = ? WHERE id = ?').run(new Date(Date.now() - 1000).toISOString(), e.body.invitation.id);
    const late = await json(await accept.POST(await req('POST', '/api/invitations/accept', { body: { token: decodeURIComponent(e.body.link.split('/invite/')[1]), password: 'correct-horse-battery' } })));
    expect(late).toMatchObject({ status: 410, body: { error: 'This invitation is expired' } });
    // Revoked.
    const v = await json<{ invitation: { id: string }; link: string }>(await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'gone@corp.example' } })));
    expect((await invitationById.DELETE(await req('DELETE', `/api/invitations/${v.body.invitation.id}`, { as: admin }), P(v.body.invitation.id))).status).toBe(200);
    expect((await invitationById.DELETE(await req('DELETE', `/api/invitations/${v.body.invitation.id}`, { as: admin }), P(v.body.invitation.id))).status).toBe(404);
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { body: { token: decodeURIComponent(v.body.link.split('/invite/')[1]), password: 'correct-horse-battery' } }))).status).toBe(410);
    // Unknown tokens count against the enrollment limiter.
    for (let i = 0; i < 20; i++) await accept.POST(await req('POST', '/api/invitations/accept', { body: { token: `bogus${i}`, password: 'correct-horse-battery' } }));
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { body: { token: 'x', password: 'correct-horse-battery' } }))).status).toBe(429);
  });

  it('attaches an existing account signed in with the invited address', async () => {
    const other = createOrg({ name: 'Other' });
    const otherAdmin = makeUser('admin@other.org', 'org_admin', other.id);
    const existing = makeUser('pat@x.org', 'org_owner');
    const wrong = makeUser('someone@x.org', 'operator');
    const r = await json<{ link: string }>(await users.POST(await req('POST', '/api/users', { as: otherAdmin, body: { email: 'pat@x.org', role: 'approver' } })));
    const token = decodeURIComponent(r.body.link.split('/invite/')[1]);
    // A new-account acceptance for an existing address is refused.
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, password: 'correct-horse-battery' } }))).status).toBe(409);
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { as: wrong, body: { token } }))).status).toBe(403);
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { as: existing, body: { token } }))).status).toBe(200);
    expect(effectiveRole(existing.id, other.id)).toBe('approver');
    expect(effectiveRole(existing.id, 'org_default')).toBe('org_owner');
  });

  it('does not demote an existing member who accepts a group invitation', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const admin = makeUser('admin@x.org', 'org_admin');
    const g = createGroup('org_default', { name: 'Readers' });
    setGroupMember(g.id, admin.id, 'owner');
    // An existing member may be invited into a group; the invitation carries the default `user` role.
    const r = await json<{ link: string }>(await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'owner@x.org', groupId: g.id } })));
    expect(r.status).toBe(201);
    const token = decodeURIComponent(r.body.link.split('/invite/')[1]);
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { as: owner, body: { token } }))).status).toBe(200);
    // Regression: accepting used to overwrite the membership with `user`.
    expect(effectiveRole(owner.id, 'org_default')).toBe('org_owner');
    expect(membersOf('org_default').find((m) => m.userId === owner.id)?.role).toBe('org_owner');
  });
});

describe('bulk user actions', () => {
  it('sets roles, removes and deactivates within the rules', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const owner2 = makeUser('owner2@x.org', 'org_owner');
    const admin = makeUser('admin@x.org', 'org_admin');
    const a = makeUser('a@x.org', 'operator');
    const b = makeUser('b@x.org', 'user');
    const bulk = async (as: typeof owner, body: unknown) => users.PATCH(await req('PATCH', '/api/users', { as, body }));
    expect((await bulk(admin, { action: 'set_role', userIds: [a.id, b.id], role: 'approver' })).status).toBe(200);
    expect(membersOf('org_default').filter((m) => m.role === 'approver')).toHaveLength(2);
    // An admin can neither demote an owner nor promote to admin.
    expect((await bulk(admin, { action: 'set_role', userIds: [owner.id], role: 'user' })).status).toBe(403);
    expect((await bulk(admin, { action: 'set_role', userIds: [a.id], role: 'org_admin' })).status).toBe(403);
    expect((await bulk(admin, { action: 'deactivate', userIds: [owner.id] })).status).toBe(403);
    expect((await bulk(admin, { action: 'set_role', userIds: [admin.id], role: 'user' })).status).toBe(403);
    // Admins lack users.remove.
    expect((await bulk(admin, { action: 'remove', userIds: [b.id] })).status).toBe(403);
    expect((await bulk(owner, { action: 'remove', userIds: [b.id] })).status).toBe(200);
    expect(effectiveRole(b.id, 'org_default')).toBeNull();
    expect((await bulk(owner, { action: 'remove', userIds: ['usr_nope'] })).status).toBe(404);
    expect((await bulk(admin, { action: 'deactivate', userIds: [a.id] })).status).toBe(200);
    expect(getUser(a.id)!.isActive).toBe(false);
    expect((await bulk(admin, { action: 'activate', userIds: [a.id] })).status).toBe(200);
    // Owners can demote another owner, but never the last one.
    expect((await bulk(owner, { action: 'set_role', userIds: [owner2.id], role: 'org_admin' })).status).toBe(200);
    expect((await bulk(owner, { action: 'bogus', userIds: [a.id] })).status).toBe(400);
    const types = listAudit({ orgIds: ['org_default'] }).entries.map((e) => e.eventType);
    expect(types).toEqual(expect.arrayContaining(['admin.role_change', 'admin.user_remove', 'admin.user_deactivate', 'admin.user_activate']));
  });

  it('does not let a sub-organization admin deactivate an account that belongs to other organizations', async () => {
    const child = createOrg({ name: 'Child', parentId: 'org_default' });
    const parentOwner = makeUser('boss@x.org', 'org_owner');
    // The parent's owner also holds a low direct role in the child.
    setMembership(parentOwner.id, child.id, 'user');
    const childAdmin = makeUser('ca@x.org', 'org_admin', child.id);
    const local = makeUser('local@x.org', 'operator', child.id);
    const res = await users.PATCH(await req('PATCH', '/api/users', { as: childAdmin, body: { action: 'deactivate', userIds: [parentOwner.id] } }));
    expect(res.status).toBe(403);
    expect(getUser(parentOwner.id)!.isActive).toBe(true);
    expect((await users.PATCH(await req('PATCH', '/api/users', { as: childAdmin, body: { action: 'deactivate', userIds: [local.id] } }))).status).toBe(200);
    // Removing the membership (local to this organization) stays possible for owners.
    // Nor an account that holds a role in an unrelated organization.
    const other = createOrg({ name: 'Unrelated' });
    const outsider = makeUser('outsider@x.org', 'org_owner', other.id);
    setMembership(outsider.id, child.id, 'user');
    expect((await users.PATCH(await req('PATCH', '/api/users', { as: childAdmin, body: { action: 'deactivate', userIds: [outsider.id] } }))).status).toBe(403);
    expect(getUser(outsider.id)!.isActive).toBe(true);
    const childOwner = makeUser('co@x.org', 'org_owner', child.id);
    expect((await users.PATCH(await req('PATCH', '/api/users', { as: childOwner, body: { action: 'remove', userIds: [parentOwner.id] } }))).status).toBe(200);
  });
});

describe('audit trail of tenancy actions', () => {
  it('records every mutation and the chain still verifies', async () => {
    const owner = makeUser('owner@x.org', 'org_owner');
    const child = (await json<{ organization: { id: string } }>(await orgs.POST(await req('POST', '/api/orgs', { as: owner, body: { name: 'C' } })))).body.organization.id;
    await orgById.PATCH(await req('PATCH', `/api/orgs/${child}`, { as: owner, body: { name: 'C2' } }), P(child));
    const g = (await json<{ group: { id: string } }>(await groups.POST(await req('POST', '/api/groups', { as: owner, body: { name: 'G' } })))).body.group.id;
    const m = makeUser('m@x.org', 'operator');
    await groupMembers.POST(await req('POST', `/api/groups/${g}/members`, { as: owner, body: { userId: m.id } }), P(g));
    await groupMembers.PATCH(await req('PATCH', `/api/groups/${g}/members`, { as: owner, body: { userId: m.id, role: 'owner' } }), P(g));
    await groupMembers.DELETE(await req('DELETE', `/api/groups/${g}/members?userId=${m.id}`, { as: owner }), P(g));
    await groupGrants.PUT(await req('PUT', `/api/groups/${g}/grants`, { as: owner, body: { grants: [] } }), P(g));
    await groupById.PATCH(await req('PATCH', `/api/groups/${g}`, { as: owner, body: { name: 'G2' } }), P(g));
    await groupById.DELETE(await req('DELETE', `/api/groups/${g}`, { as: owner }), P(g));
    const inv = await json<{ invitation: { id: string } }>(await users.POST(await req('POST', '/api/users', { as: owner, body: { email: 'z@x.org' } })));
    await invitationById.DELETE(await req('DELETE', `/api/invitations/${inv.body.invitation.id}`, { as: owner }), P(inv.body.invitation.id));
    await orgSwitch.POST(await req('POST', '/api/orgs/switch', { as: owner, body: { orgId: child } }));
    const types = listAudit({}).entries.map((e) => e.eventType);
    expect(types).toEqual(expect.arrayContaining(['org.create', 'org.update', 'group.create', 'group.member_add', 'group.member_role', 'group.member_remove', 'group.grants', 'group.update', 'group.delete', 'admin.user_invite', 'admin.invite_revoke', 'org.switch']));
    expect(verifyAudit()).toMatchObject({ ok: true });
  });
});
