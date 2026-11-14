/** Console API tokens: scoped to a subset of the owner's permissions, expiring, revocable, never for account changes. */
import { describe, expect, it } from 'vitest';
import { NextRequest } from 'next/server';
import { middleware } from '@/middleware';
import { db } from '@/lib/db';
import { createOrg, setMembership } from '@/lib/db/repos/orgs';
import { setSecurity } from '@/lib/db/repos/security';
import { listAudit } from '@/lib/db/repos/audit';
import { json, makeUser, req } from './helpers';
import * as tokens from '@/app/api/api-tokens/route';
import * as tokenById from '@/app/api/api-tokens/[id]/route';
import * as users from '@/app/api/users/route';
import * as security from '@/app/api/security/route';
import * as passkeys from '@/app/api/account/passkeys/route';

type U = ReturnType<typeof makeUser>;
const bearer = (secret: string, extra: Record<string, string> = {}) => ({
  authorization: `Bearer ${secret}`,
  ...extra,
});
const mint = async (u: U, permissions: unknown, expiresInDays: unknown = 30) =>
  json<{
    secret: string;
    token: { id: string; permissions: string[] };
    error?: string;
  }>(
    await tokens.POST(
      await req('POST', '/api/api-tokens', {
        as: u,
        body: { name: 'script', permissions, expiresInDays },
      }),
    ),
  );

describe('console API tokens', () => {
  it('carries only the chosen permissions, never more than the owner has', async () => {
    const org = createOrg({ name: 'Token org' });
    const admin = makeUser('tok-admin@x.org', 'org_admin', org.id);
    expect((await mint(admin, ['sso.configure'])).body.error).toMatch(/do not have: sso.configure/);
    expect((await mint(admin, [])).status).toBe(400);
    expect((await mint(admin, ['users.read'], 366)).status).toBe(400);
    const t = await mint(admin, ['admin.access', 'users.read']);
    expect(t.status).toBe(201);
    expect(t.body.secret).toMatch(/^ufop_cat_/);
    expect(JSON.stringify(db().prepare('SELECT * FROM api_tokens').all())).not.toContain(t.body.secret);

    expect((await users.GET(await req('GET', '/api/users', { headers: bearer(t.body.secret) }))).status).toBe(200);
    // users.write is not in the token, so bulk changes are refused.
    expect(
      (
        await users.PATCH(
          await req('PATCH', '/api/users', {
            headers: bearer(t.body.secret),
            body: { action: 'sign_out', userIds: ['x'] },
          }),
        )
      ).status,
    ).toBe(403);
    const list = await json<{
      tokens: { lastUsedAt: string | null; lastUsedIp: string | null }[];
    }>(await tokens.GET(await req('GET', '/api/api-tokens', { as: admin })));
    expect(list.body.tokens[0].lastUsedAt).toBeTruthy();
    expect(list.body.tokens[0].lastUsedIp).toBe('203.0.113.7');
    expect(listAudit({ eventType: 'api_token.create' }).entries[0].description).toContain('2 permissions');

    // Demoting the owner narrows the token right away.
    setMembership(admin.id, org.id, 'user');
    expect((await users.GET(await req('GET', '/api/users', { headers: bearer(t.body.secret) }))).status).toBe(403);
  });

  it('cannot do account, sign-in or step-up changes, or mint more tokens', async () => {
    const org = createOrg({ name: 'Token limits' });
    const sec = makeUser('tok-sec@x.org', 'security_admin', org.id);
    const t = await mint(sec, ['admin.access', 'users.read', 'sso.configure']);
    const put = await json<{ code: string }>(
      await security.PUT(
        await req('PUT', '/api/security', {
          headers: bearer(t.body.secret),
          body: { sessionIdleMinutes: 30 },
        }),
      ),
    );
    expect(put.status).toBe(403);
    expect(put.body.code).toBe('interactive_required');
    expect(
      (
        await tokens.POST(
          await req('POST', '/api/api-tokens', {
            headers: bearer(t.body.secret),
            body: { name: 'x', permissions: ['users.read'], expiresInDays: 1 },
          }),
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await passkeys.GET(
          await req('GET', '/api/account/passkeys', {
            headers: bearer(t.body.secret),
          }),
        )
      ).status,
    ).toBe(403);
    // The organization's network rules apply to tokens too.
    setSecurity(org.id, { ipAllowlist: ['10.0.0.0/8'] }, 't');
    expect((await security.GET(await req('GET', '/api/security', { headers: bearer(t.body.secret) }))).status).toBe(403);
  });

  it('stops working when revoked, expired or the owner is deactivated', async () => {
    const org = createOrg({ name: 'Token life' });
    const admin = makeUser('tok-life@x.org', 'org_admin', org.id);
    const other = makeUser('tok-other@x.org', 'operator', org.id);
    const a = await mint(admin, ['admin.access', 'users.read']);
    const b = await mint(other, ['admin.access', 'fleet.read']);
    const get = async (s: string) => (await users.GET(await req('GET', '/api/users', { headers: bearer(s) }))).status;
    // A member cannot revoke someone else's token; an administrator can.
    expect(
      (
        await tokenById.DELETE(
          await req('DELETE', `/api/api-tokens/${a.body.token.id}`, {
            as: other,
          }),
          { params: Promise.resolve({ id: a.body.token.id }) },
        )
      ).status,
    ).toBe(404);
    const all = await json<{ tokens: unknown[] }>(await tokens.GET(await req('GET', '/api/api-tokens?all=1', { as: admin })));
    expect(all.body.tokens).toHaveLength(2);
    expect(
      (
        await tokenById.DELETE(
          await req('DELETE', `/api/api-tokens/${b.body.token.id}`, {
            as: admin,
          }),
          { params: Promise.resolve({ id: b.body.token.id }) },
        )
      ).status,
    ).toBe(200);
    expect(await get(b.body.secret)).toBe(401);

    db()
      .prepare('UPDATE api_tokens SET expires_at = ? WHERE id = ?')
      .run(new Date(Date.now() - 1000).toISOString(), a.body.token.id);
    expect(await get(a.body.secret)).toBe(401);
    const c = await mint(admin, ['admin.access', 'users.read']);
    expect(await get(c.body.secret)).toBe(200);
    const owner = makeUser('tok-owner@x.org', 'org_owner', org.id);
    expect(
      (
        await users.PATCH(
          await req('PATCH', '/api/users', {
            as: owner,
            body: { action: 'deactivate', userIds: [admin.id] },
          }),
        )
      ).status,
    ).toBe(200);
    expect(await get(c.body.secret)).toBe(401);
    expect(db().prepare('SELECT revoked_at FROM api_tokens WHERE id = ?').get(c.body.token.id)).toMatchObject({ revoked_at: expect.any(String) });
  });

  it('passes bearer-token API calls through the middleware only without a session cookie', async () => {
    const post = (headers: Record<string, string>) =>
      middleware(
        new NextRequest(new URL('http://localhost:3001/api/fleet/jobs'), {
          method: 'POST',
          headers,
        }),
      );
    // Scripts send no Origin; with a token and no cookie the route authenticates.
    expect((await post({ authorization: 'Bearer ufop_cat_x' })).headers.get('x-middleware-next')).toBe('1');
    // With a cookie, the CSRF check still applies.
    expect(
      (
        await post({
          authorization: 'Bearer ufop_cat_x',
          cookie: 'ufop-admin-session=abc',
        })
      ).status,
    ).toBe(403);
    expect((await post({ authorization: 'Bearer other' })).status).toBe(403);
  });
});
