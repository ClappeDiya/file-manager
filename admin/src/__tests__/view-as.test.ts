/** Read-only "view as": who may, what it shows, what it refuses, expiry and the audit trail. */
import { describe, expect, it } from 'vitest';
import { db } from '@/lib/db';
import { createSession } from '@/lib/db/repos/sessions';
import { createOrg } from '@/lib/db/repos/orgs';
import { listAudit } from '@/lib/db/repos/audit';
import { listApiRequests } from '@/lib/db/repos/api-log';
import { json, makeUser, req } from './helpers';
import * as viewAs from '@/app/api/view-as/route';
import * as session from '@/app/api/auth/session/route';
import * as managedSettings from '@/app/api/fleet/managed-settings/route';

describe('view as', () => {
  it('shows the console as a member, read-only, and audits it', async () => {
    const admin = makeUser('va-admin@x.org', 'org_admin');
    const member = makeUser('va-member@x.org', 'user');
    const owner = makeUser('va-owner@x.org', 'org_owner');
    const outsider = makeUser('va-out@x.org', 'user', createOrg({ name: 'Elsewhere' }).id);
    const sid = createSession(admin.id, { hours: 8 }).id;
    const as = { as: admin, sid };
    const start = async (userId: string, minutes?: number) => viewAs.POST(await req('POST', '/api/view-as', { ...as, body: { userId, minutes } }));

    expect((await start(admin.id)).status).toBe(400);
    expect((await start(owner.id)).status).toBe(403);
    expect((await start(outsider.id)).status).toBe(404);
    expect((await viewAs.POST(await req('POST', '/api/view-as', { as: member, body: { userId: admin.id } }))).status).toBe(403); // needs user management

    const started = await json<{ viewAs: { user: { email: string }; role: string; until: string } }>(await start(member.id, 90));
    expect(started.status).toBe(200);
    expect(started.body.viewAs.user.email).toBe('va-member@x.org');
    expect(Date.parse(started.body.viewAs.until) - Date.now()).toBeLessThanOrEqual(60 * 60_000 + 1000);

    // The session now reads as the member, with read permissions only.
    const s = await json<{ user: { email: string }; role: string; permissions: string[]; viewAs: { actor: { email: string } } }>(await session.GET(await req('GET', '/api/auth/session', as)));
    expect(s.body.user.email).toBe('va-member@x.org');
    expect(s.body.role).toBe('user');
    expect(s.body.viewAs.actor.email).toBe('va-admin@x.org');
    expect(s.body.permissions.every((p) => p === 'admin.access' || p.endsWith('.read'))).toBe(true);
    // Changes are refused, whatever the member could do.
    const put = await json<{ code: string }>(await managedSettings.PUT(await req('PUT', '/api/fleet/managed-settings', { ...as, body: { ai: { enabled: false } } })));
    expect(put.status).toBe(403);
    expect(put.body.code).toBe('view_as_read_only');
    // Starting another needs stopping first (and the member's permissions don't allow it anyway).
    expect((await start(member.id)).status).toBe(403);

    expect((await json<{ viewAs: null }>(await viewAs.DELETE(await req('DELETE', '/api/view-as', as)))).body.viewAs).toBeNull();
    const back = await json<{ user: { email: string }; viewAs: null }>(await session.GET(await req('GET', '/api/auth/session', as)));
    expect(back.body.user.email).toBe('va-admin@x.org');
    expect(back.body.viewAs).toBeNull();

    const events = listAudit({}).entries.filter((e) => e.eventType.startsWith('user.view_as'));
    expect(events.map((e) => [e.eventType, e.userEmail])).toEqual(expect.arrayContaining([['user.view_as_start', 'va-admin@x.org'], ['user.view_as_stop', 'va-admin@x.org']]));
    const logged = listApiRequests('org_default', { limit: 200 }).requests.filter((r) => r.outcome === 'viewing as va-member@x.org');
    expect(logged.length).toBeGreaterThanOrEqual(2);
    expect(logged.every((r) => r.userEmail === 'va-admin@x.org')).toBe(true);
  });

  it('ends by itself at the time limit', async () => {
    const admin = makeUser('va-admin2@x.org', 'org_admin');
    const member = makeUser('va-member2@x.org', 'read_only_auditor');
    const sid = createSession(admin.id, { hours: 8 }).id;
    expect((await viewAs.POST(await req('POST', '/api/view-as', { as: admin, sid, body: { userId: member.id } }))).status).toBe(200);
    db().prepare('UPDATE sessions SET view_as_until = ? WHERE id = ?').run(new Date(Date.now() - 1000).toISOString(), sid);
    const s = await json<{ user: { email: string }; viewAs: null }>(await session.GET(await req('GET', '/api/auth/session', { as: admin, sid })));
    expect(s.body.user.email).toBe('va-admin2@x.org');
    expect(listAudit({}).entries.some((e) => e.eventType === 'user.view_as_end' && e.description.includes('time limit'))).toBe(true);
  });
});
