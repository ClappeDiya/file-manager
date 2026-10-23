/** Users → Explain: the effective-access explanation matches enforcement. */
import { describe, expect, it } from 'vitest';
import { createOrg, effectiveRole, setMembership } from '@/lib/db/repos/orgs';
import { createGroup, setGroupGrants, setGroupMember } from '@/lib/db/repos/groups';
import { setSecurity } from '@/lib/db/repos/security';
import { explainAccess } from '@/lib/explain';
import { updateUser } from '@/lib/db/repos/users';
import { json, makeUser, req } from './helpers';
import * as access from '@/app/api/users/[id]/access/route';

const call = async (as: ReturnType<typeof makeUser>, id: string, q = '', org?: string) =>
  json<Record<string, unknown> & { error?: string }>(await access.GET(await req('GET', `/api/users/${id}/access${q}`, { as, org }), { params: Promise.resolve({ id }) }));

describe('explain access', () => {
  it('lists every grant, marks which apply and why not, and agrees with effectiveRole', () => {
    const parent = createOrg({ name: 'Parent Co' });
    const child = createOrg({ name: 'Child Co', parentId: parent.id });
    const island = createOrg({ name: 'Island', parentId: parent.id, detached: true });
    const u = makeUser('explain@x.org', 'operator', parent.id);
    const delegating = createGroup(parent.id, { name: 'Admins', orgRole: 'org_admin', delegateToChildren: true });
    const local = createGroup(parent.id, { name: 'Local auditors', orgRole: 'read_only_auditor', delegateToChildren: false });
    setGroupMember(delegating.id, u.id, 'member');
    setGroupMember(local.id, u.id, 'member');
    setMembership(u.id, child.id, 'user');

    const e = explainAccess(u.id, child.id, 'users.write')!;
    expect(e.role).toBe('org_admin');
    expect(e.role).toBe(effectiveRole(u.id, child.id));
    expect(e.sources).toEqual([
      expect.objectContaining({ via: 'membership', orgName: 'Child Co', role: 'user', inherited: false, applies: true }),
      expect.objectContaining({ via: 'membership', orgName: 'Parent Co', role: 'operator', inherited: true, applies: true }),
      expect.objectContaining({ via: 'group', groupName: 'Admins', role: 'org_admin', inherited: true, applies: true }),
      expect.objectContaining({ via: 'group', groupName: 'Local auditors', applies: false, note: expect.stringMatching(/does not delegate/) }),
    ]);
    expect(e.check).toEqual({ permission: 'users.write', allowed: true, reason: expect.stringContaining('group "Admins" in parent organization "Parent Co"') });

    // A detached child: nothing from above applies.
    const d = explainAccess(u.id, island.id, 'fleet.read')!;
    expect(d.role).toBeNull();
    expect(effectiveRole(u.id, island.id)).toBeNull();
    expect(d.sources.every((s) => !s.applies && /detached/.test(s.note ?? ''))).toBe(true);
    expect(d.check).toMatchObject({ allowed: false, reason: 'No role applies in this organization' });
  });

  it('shows machine scoping, sign-in rules and refuses people outside the organization', async () => {
    const org = createOrg({ name: 'Scoped org' });
    const admin = makeUser('exp-admin@x.org', 'org_admin', org.id);
    const owner = makeUser('exp-user@x.org', 'user', org.id);
    const g = createGroup(org.id, { name: 'Laptops' });
    setGroupMember(g.id, owner.id, 'member');
    setGroupGrants(g.id, [{ resourceType: 'tag', resourceId: 'laptop' }]);
    updateUser(admin.id, { mfaEnabled: true }); // the viewer satisfies the rule; the member does not
    setSecurity(org.id, { requireMfa: true, ipAllowlist: ['203.0.113.0/24'] }, 't');
    makeUser('stranger@x.org', 'operator', createOrg({ name: 'Elsewhere' }).id);

    const r = await call(admin, owner.id, '?permission=fleet.write');
    expect(r.status).toBe(200);
    expect(r.body).toMatchObject({ role: 'user', scoped: true, grants: [{ resourceType: 'tag', resourceId: 'laptop', groupName: 'Laptops' }] });
    const conditions = r.body.conditions as { key: string; ok: boolean; detail: string }[];
    expect(conditions.find((c) => c.key === 'two_factor')).toMatchObject({ ok: false });
    expect(conditions.find((c) => c.key === 'networks')!.detail).toContain('203.0.113.0/24');
    expect(r.body.check).toMatchObject({ allowed: false, reason: 'user does not include fleet.write' });
    expect((await call(admin, owner.id, '?permission=nonsense')).status).toBe(400);
  });

  it('needs users.read for someone else, not for yourself', async () => {
    const org = createOrg({ name: 'Self org' });
    const approver = makeUser('exp-approver@x.org', 'approver', org.id);
    const other = makeUser('exp-other@x.org', 'operator', org.id);
    expect((await call(approver, approver.id)).status).toBe(200);
    expect((await call(approver, other.id)).status).toBe(403);
    const admin = makeUser('exp-admin2@x.org', 'org_admin', org.id);
    const outsider = makeUser('exp-out@x.org', 'operator', createOrg({ name: 'Out' }).id);
    expect((await call(admin, outsider.id)).status).toBe(404);
  });
});
