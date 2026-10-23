/** API request log: token traffic with outcomes, refused requests, org scoping and retention. */
import { describe, expect, it } from 'vitest';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { createApiToken, revokeApiToken } from '@/lib/db/repos/api-tokens';
import { setSecurity } from '@/lib/db/repos/security';
import { applyRetention } from '@/lib/retention';
import { json, makeUser, req } from './helpers';
import * as users from '@/app/api/users/route';
import * as apiRequests from '@/app/api/api-requests/route';

describe('API request log', () => {
  it('logs token requests with their outcome, refused ones and network refusals, per organization', async () => {
    const org = createOrg({ name: 'Log org' });
    const admin = makeUser('log-admin@x.org', 'org_admin', org.id);
    const t = createApiToken({ orgId: org.id, userId: admin.id, name: 'reader', permissions: ['admin.access', 'users.read'], days: 30, allowed: ['admin.access', 'users.read'] });
    const bearer = (s: string, ip = '203.0.113.7') => ({ authorization: `Bearer ${s}`, 'user-agent': 'report-script/1.0', 'x-forwarded-for': ip });
    expect((await users.GET(await req('GET', '/api/users', { headers: bearer(t.secret) }))).status).toBe(200);
    expect((await users.PATCH(await req('PATCH', '/api/users', { headers: bearer(t.secret), body: { action: 'sign_out', userIds: ['x'] } }))).status).toBe(403);
    revokeApiToken(t.token.id);
    expect((await users.GET(await req('GET', '/api/users', { headers: bearer(t.secret) }))).status).toBe(401);
    // A session refused by the network rule.
    setSecurity(org.id, { ipAllowlist: ['10.0.0.0/8'] }, 't');
    expect((await users.GET(await req('GET', '/api/users', { as: admin, headers: { 'x-forwarded-for': '198.51.100.9' } }))).status).toBe(403);
    setSecurity(org.id, { ipAllowlist: [] }, 't');

    const list = await json<{ requests: { method: string; path: string; status: number; outcome: string; auth: string; tokenId: string | null; userEmail: string; ip: string; userAgent: string }[]; total: number }>(
      await apiRequests.GET(await req('GET', '/api/api-requests', { as: admin })),
    );
    expect(list.status).toBe(200);
    expect(list.body.requests.map((r) => [r.auth, r.method, r.status, r.outcome])).toEqual([
      ['session', 'GET', 403, 'network not allowed'],
      ['token', 'GET', 401, 'unknown, expired or revoked token'],
      ['token', 'PATCH', 403, 'missing users.deactivate'],
      ['token', 'GET', 200, 'authorized'],
    ]);
    expect(list.body.requests[3]).toMatchObject({ path: '/api/users', tokenId: t.token.id, userEmail: 'log-admin@x.org', ip: '203.0.113.7', userAgent: 'report-script/1.0' });
    expect(list.body.requests[1].tokenId).toBe(t.token.id); // a revoked token is still attributed
    const refused = await json<{ total: number }>(await apiRequests.GET(await req('GET', '/api/api-requests?result=refused', { as: admin })));
    expect(refused.body.total).toBe(3);
    const byToken = await json<{ total: number }>(await apiRequests.GET(await req('GET', `/api/api-requests?tokenId=${t.token.id}`, { as: admin })));
    expect(byToken.body.total).toBe(3);

    // Unknown tokens (no organization) and other organizations stay out of this view.
    await users.GET(await req('GET', '/api/users', { headers: bearer('ufop_cat_nonsense') }));
    const other = makeUser('log-other@x.org', 'org_admin', createOrg({ name: 'Log other' }).id);
    expect((await json<{ total: number }>(await apiRequests.GET(await req('GET', '/api/api-requests', { as: other })))).body.total).toBe(0);
    expect((await json<{ total: number }>(await apiRequests.GET(await req('GET', '/api/api-requests', { as: admin })))).body.total).toBe(4);

    // Retention.
    db()
      .prepare('UPDATE api_requests SET at = ? WHERE org_id = ?')
      .run(new Date(Date.now() - 31 * 86_400_000).toISOString(), org.id);
    expect(applyRetention().api_requests).toBeGreaterThanOrEqual(4);
    expect((await json<{ total: number }>(await apiRequests.GET(await req('GET', '/api/api-requests', { as: admin })))).body.total).toBe(0);
  });
});
