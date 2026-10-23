/** Retiring members and API tokens without activity (Settings → Security → Inactive access). */
import { describe, expect, it } from 'vitest';
import { db } from '@/lib/db';
import { createOrg, membershipsOf, setMembership } from '@/lib/db/repos/orgs';
import { setSecurity } from '@/lib/db/repos/security';
import { getUser, updateUser } from '@/lib/db/repos/users';
import { createApiToken, getApiToken } from '@/lib/db/repos/api-tokens';
import { createSession } from '@/lib/db/repos/sessions';
import { listAudit } from '@/lib/db/repos/audit';
import { retireInactive } from '@/lib/retirement';
import { json, makeUser, req } from './helpers';
import * as security from '@/app/api/security/route';

const DAY = 86_400_000;
const ago = (days: number) => new Date(Date.now() - days * DAY).toISOString();
const joined = (userId: string, orgId: string, days: number) => db().prepare('UPDATE org_memberships SET created_at = ? WHERE user_id = ? AND org_id = ?').run(ago(days), userId, orgId);

describe('inactive access retirement', () => {
  it('validates the setting', async () => {
    const org = createOrg({ name: 'Retire settings' });
    const sec = makeUser('ret-sec@x.org', 'security_admin', org.id);
    const put = async (retireInactiveDays: number) => (await security.PUT(await req('PUT', '/api/security', { as: sec, body: { retireInactiveDays } }))).status;
    expect(await put(10)).toBe(400);
    expect(await put(4000)).toBe(400);
    expect(await put(90)).toBe(200);
    expect((await json<{ settings: { retireInactiveDays: number } }>(await security.GET(await req('GET', '/api/security', { as: sec })))).body.settings.retireInactiveDays).toBe(90);
  });

  it('removes idle members, keeps active ones and owners, and disables single-org accounts', () => {
    const org = createOrg({ name: 'Retire org' });
    const other = createOrg({ name: 'Retire other' });
    const owner = makeUser('ret-owner@x.org', 'org_owner', org.id);
    const idle = makeUser('ret-idle@x.org', 'operator', org.id);
    const multi = makeUser('ret-multi@x.org', 'operator', org.id);
    setMembership(multi.id, other.id, 'operator');
    const recentLogin = makeUser('ret-login@x.org', 'operator', org.id);
    const recentSession = makeUser('ret-session@x.org', 'operator', org.id);
    const recentToken = makeUser('ret-token@x.org', 'org_admin', org.id);
    const newcomer = makeUser('ret-new@x.org', 'operator', org.id);
    for (const u of [owner, idle, multi, recentLogin, recentSession, recentToken]) joined(u.id, org.id, 200);
    joined(newcomer.id, org.id, 10);
    updateUser(idle.id, { lastLoginAt: ago(120) });
    updateUser(recentLogin.id, { lastLoginAt: ago(5) });
    const s = createSession(recentSession.id, { hours: 8 });
    db().prepare('UPDATE sessions SET last_seen_at = ? WHERE id = ?').run(ago(3), s.id);
    const used = createApiToken({ orgId: org.id, userId: recentToken.id, name: 'cron', permissions: ['users.read'], days: 365, allowed: ['users.read'] });
    db().prepare('UPDATE api_tokens SET created_at = ?, last_used_at = ? WHERE id = ?').run(ago(200), ago(1), used.token.id);
    const stale = createApiToken({ orgId: org.id, userId: recentLogin.id, name: 'old script', permissions: ['users.read'], days: 365, allowed: ['users.read'] });
    db().prepare('UPDATE api_tokens SET created_at = ? WHERE id = ?').run(ago(100), stale.token.id);

    // Off by default: nothing happens.
    expect(retireInactive()).toEqual({ members: [], tokens: [] });
    setSecurity(org.id, { retireInactiveDays: 90 }, 't');
    const r = retireInactive();
    expect(r.members.map((m) => m.email).sort()).toEqual(['ret-idle@x.org', 'ret-multi@x.org']);
    expect(r.tokens.map((t) => t.name)).toEqual(['old script']);
    expect(getUser(idle.id)!.isActive).toBe(false);
    // Still a member elsewhere: removed here only, the account keeps working.
    expect(getUser(multi.id)!.isActive).toBe(true);
    expect(membershipsOf(multi.id).map((m) => m.orgId)).toEqual([other.id]);
    expect(membershipsOf(owner.id)).toHaveLength(1);
    expect(getApiToken(stale.token.id)!.revokedAt).toBeTruthy();
    expect(getApiToken(used.token.id)!.revokedAt).toBeNull();
    expect(listAudit({ eventType: 'user.retired_inactive' }).entries.some((e) => e.description.includes('ret-idle@x.org') && e.description.includes('account disabled'))).toBe(true);
    expect(listAudit({ eventType: 'api_token.retired_inactive' }).entries[0].description).toContain('old script');
    // Idempotent.
    expect(retireInactive()).toEqual({ members: [], tokens: [] });
  });
});
