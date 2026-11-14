/** Removing members reversibly: successor hand-over, restore, membership expiry and purge. */
import { describe, expect, it } from 'vitest';
import { db } from '@/lib/db';
import { createOrg, membershipsOf, setMembership } from '@/lib/db/repos/orgs';
import { createGroup, groupMembers, setGroupMember } from '@/lib/db/repos/groups';
import { createChannel, getChannel, type EmailConfig } from '@/lib/db/repos/channels';
import { createApiToken, getApiToken } from '@/lib/db/repos/api-tokens';
import { getUser } from '@/lib/db/repos/users';
import { listAudit } from '@/lib/db/repos/audit';
import { expireMemberships, listRemovedMembers, purgeRemovedMembers } from '@/lib/offboarding';
import { json, makeUser, req } from './helpers';
import * as users from '@/app/api/users/route';

const email = (to: string[]) => ({ host: 'smtp.example', port: 587, security: 'starttls', username: '', from: 'ufop@example.com', to });
const patch = async (as: ReturnType<typeof makeUser>, body: Record<string, unknown>) =>
  json<{ error?: string; results?: { groupsHandedOver: number; channelsUpdated: number; tokensRevoked: number; accountDisabled: boolean }[] }>(await users.PATCH(await req('PATCH', '/api/users', { as, body })));

describe('offboarding', () => {
  it('removes with a successor, then restores role, groups and the account', async () => {
    const org = createOrg({ name: 'Offboard org' });
    const admin = makeUser('off-admin@x.org', 'org_owner', org.id);
    const leaver = makeUser('off-leaver@x.org', 'operator', org.id);
    const heir = makeUser('off-heir@x.org', 'operator', org.id);
    const g = createGroup(org.id, { name: 'Ops' });
    setGroupMember(g.id, leaver.id, 'owner');
    const solo = createChannel(org.id, { name: 'Only leaver', kind: 'email', config: email(['off-leaver@x.org']) });
    const shared = createChannel(org.id, { name: 'Team', kind: 'email', config: email(['ops@x.org', 'OFF-LEAVER@x.org']) });
    const tok = createApiToken({ orgId: org.id, userId: leaver.id, name: 's', permissions: ['users.read'], days: 30, allowed: ['users.read'] });

    expect((await patch(admin, { action: 'remove', userIds: [leaver.id], successorId: leaver.id })).status).toBe(400);
    expect((await patch(admin, { action: 'remove', userIds: [leaver.id], successorId: 'usr_nobody' })).status).toBe(400);
    const r = await patch(admin, { action: 'remove', userIds: [leaver.id], successorId: heir.id });
    expect(r.status).toBe(200);
    expect(r.body.results![0]).toMatchObject({ groupsHandedOver: 1, channelsUpdated: 2, tokensRevoked: 1, accountDisabled: true });
    expect(groupMembers(g.id)).toEqual([expect.objectContaining({ userId: heir.id, role: 'owner' })]);
    expect((getChannel(org.id, solo.id)!.config as EmailConfig).to).toEqual(['off-heir@x.org']);
    expect((getChannel(org.id, shared.id)!.config as EmailConfig).to).toEqual(['ops@x.org', 'off-heir@x.org']);
    expect(getApiToken(tok.token.id)!.revokedAt).toBeTruthy();
    expect(getUser(leaver.id)!.isActive).toBe(false);
    const roster = await json<{ removed: { userId: string; reason: string; reassignedTo: string }[] }>(await users.GET(await req('GET', '/api/users', { as: admin })));
    expect(roster.body.removed).toEqual([expect.objectContaining({ userId: leaver.id, reason: 'removed', reassignedTo: heir.id })]);

    expect((await patch(admin, { action: 'restore', userIds: [leaver.id] })).status).toBe(200);
    expect(membershipsOf(leaver.id)).toEqual([expect.objectContaining({ orgId: org.id, role: 'operator' })]);
    expect(groupMembers(g.id).find((m) => m.userId === leaver.id)?.role).toBe('owner');
    expect(getUser(leaver.id)!.isActive).toBe(true);
    expect(listRemovedMembers(org.id)).toEqual([]);
    expect((await patch(admin, { action: 'restore', userIds: [leaver.id] })).status).toBe(404);
    expect(listAudit({ eventType: 'admin.user_restore' }).entries[0].description).toContain('off-leaver@x.org');
  });

  it('leaves the account working when it belongs to another organization, and without a successor keeps sole recipients', async () => {
    const org = createOrg({ name: 'Offboard two' });
    const other = createOrg({ name: 'Offboard other' });
    const admin = makeUser('off2-admin@x.org', 'org_owner', org.id);
    const u = makeUser('off2-u@x.org', 'operator', org.id);
    setMembership(u.id, other.id, 'operator');
    const solo = createChannel(org.id, { name: 'Solo', kind: 'email', config: email(['off2-u@x.org']) });
    const r = await patch(admin, { action: 'remove', userIds: [u.id] });
    expect(r.body.results![0]).toMatchObject({ accountDisabled: false, channelsUpdated: 0 });
    expect(getUser(u.id)!.isActive).toBe(true);
    expect((getChannel(org.id, solo.id)!.config as EmailConfig).to).toEqual(['off2-u@x.org']);
  });

  it('ends memberships on their date, never the last owner, and purges after the restore window', async () => {
    const org = createOrg({ name: 'Expiry org' });
    const owner = makeUser('exp-owner@x.org', 'org_owner', org.id);
    const temp = makeUser('exp-temp@x.org', 'operator', org.id);
    expect((await patch(owner, { action: 'set_expiry', userIds: [temp.id], expiresAt: '2001-01-01T00:00:00Z' })).status).toBe(400);
    const soon = new Date(Date.now() + 3600_000).toISOString();
    expect((await patch(owner, { action: 'set_expiry', userIds: [temp.id], expiresAt: soon })).status).toBe(200);
    const roster = await json<{ users: { id: string; expiresAt: string | null }[] }>(await users.GET(await req('GET', '/api/users', { as: owner })));
    expect(roster.body.users.find((x) => x.id === temp.id)!.expiresAt).toBe(soon);
    db()
      .prepare('UPDATE org_memberships SET expires_at = ? WHERE org_id = ?')
      .run(new Date(Date.now() - 1000).toISOString(), org.id);
    expect(expireMemberships()).toEqual([{ orgId: org.id, userId: temp.id }]);
    expect(membershipsOf(owner.id)).toHaveLength(1); // the last owner stays
    expect(listRemovedMembers(org.id)[0]).toMatchObject({ userId: temp.id, reason: 'expired', accountDisabled: true });
    expect(listAudit({ eventType: 'user.membership_expired' }).entries[0].description).toContain('exp-temp@x.org');

    expect(purgeRemovedMembers(new Date(Date.now() + 29 * 86_400_000))).toEqual([]);
    expect(purgeRemovedMembers(new Date(Date.now() + 31 * 86_400_000))).toEqual([temp.id]);
    expect(getUser(temp.id)).toBeNull();
  });
});
