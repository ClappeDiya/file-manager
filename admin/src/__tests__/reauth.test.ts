/** Step-up re-authentication before sensitive changes. */
import { describe, expect, it } from 'vitest';
import { hash } from 'bcryptjs';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { createSession, getSession } from '@/lib/db/repos/sessions';
import { updateUser } from '@/lib/db/repos/users';
import { listAudit } from '@/lib/db/repos/audit';
import { seal } from '@/lib/crypto';
import { hotp, newTotpSecret, totpStep } from '@/lib/totp';
import { json, makeUser, req } from './helpers';
import { Authenticator } from './webauthn-helpers';
import * as reauth from '@/app/api/auth/reauth/route';
import * as security from '@/app/api/security/route';
import * as users from '@/app/api/users/route';
import * as passkeys from '@/app/api/account/passkeys/route';

type U = ReturnType<typeof makeUser>;

/** A session that signed in an hour ago. */
function staleSession(u: U): string {
  const s = createSession(u.id, { hours: 8 });
  const old = new Date(Date.now() - 3600_000).toISOString();
  db().prepare('UPDATE sessions SET created_at = ?, verified_at = ? WHERE id = ?').run(old, old, s.id);
  return s.id;
}

const putSecurity = async (u: U, sid: string) =>
  json<{ code?: string; methods?: string[] }>(
    await security.PUT(
      await req('PUT', '/api/security', {
        as: u,
        sid,
        body: { sessionIdleMinutes: 30 },
      }),
    ),
  );
const confirm = async (u: U, sid: string, body: unknown) =>
  json<{
    ok?: boolean;
    error?: string;
    challengeId?: string;
    publicKey?: { challenge: string; allowCredentials: unknown[] };
  }>(await reauth.POST(await req('POST', '/api/auth/reauth', { as: u, sid, body })));

describe('step-up re-authentication', () => {
  it('a fresh sign-in may make sensitive changes; an old one must confirm the password', async () => {
    const org = createOrg({ name: 'Step-up org' });
    const u = makeUser('stepup@x.org', 'security_admin', org.id, {
      passwordHash: await hash('Step-Up-Pass-1', 4),
    });
    expect(
      (
        await security.PUT(
          await req('PUT', '/api/security', {
            as: u,
            body: { sessionIdleMinutes: 30 },
          }),
        )
      ).status,
    ).toBe(200);
    const sid = staleSession(u);
    const refused = await putSecurity(u, sid);
    expect(refused.status).toBe(403);
    expect(refused.body).toMatchObject({
      code: 'reauth_required',
      methods: ['password'],
    });
    // Reading is not sensitive.
    expect((await security.GET(await req('GET', '/api/security', { as: u, sid }))).status).toBe(200);

    expect((await confirm(u, sid, { method: 'password', password: 'wrong' })).status).toBe(401);
    expect(listAudit({ eventType: 'auth.reauth_failed' }).entries[0].userEmail).toBe(u.email);
    expect((await confirm(u, sid, { method: 'totp', code: '123456' })).status).toBe(400); // not this account's method
    expect(
      (
        await confirm(u, sid, {
          method: 'password',
          password: 'Step-Up-Pass-1',
        })
      ).status,
    ).toBe(200);
    expect(new Date(getSession(sid)!.verifiedAt).getTime()).toBeGreaterThan(Date.now() - 5000);
    expect((await putSecurity(u, sid)).status).toBe(200);
    expect(listAudit({ eventType: 'auth.reauth' }).entries[0].description).toContain('password');
  });

  it('accounts with two-factor sign-in confirm with a code, not just the password', async () => {
    const org = createOrg({ name: 'Step-up TOTP' });
    const secret = newTotpSecret();
    const u = makeUser('stepup-totp@x.org', 'security_admin', org.id, {
      passwordHash: await hash('Step-Up-Pass-2', 4),
    });
    updateUser(u.id, { mfaEnabled: true, totpSecretCiphertext: seal(secret) });
    const fresh = {
      ...u,
      mfaEnabled: true,
      totpSecretCiphertext: seal(secret),
    };
    const sid = staleSession(fresh);
    expect((await putSecurity(fresh, sid)).body.methods).toEqual(['totp']);
    expect(
      (
        await confirm(fresh, sid, {
          method: 'password',
          password: 'Step-Up-Pass-2',
        })
      ).status,
    ).toBe(400);
    const code = hotp(secret, totpStep());
    expect((await confirm(fresh, sid, { method: 'totp', code })).status).toBe(200);
    // The same code cannot be replayed.
    const sid2 = staleSession(fresh);
    expect((await confirm(fresh, sid2, { method: 'totp', code })).status).toBe(401);
  });

  it('confirms with a passkey only with user verification, and only the account’s own', async () => {
    const org = createOrg({ name: 'Step-up passkey' });
    const u = makeUser('stepup-pk@x.org', 'org_admin', org.id);
    const a = new Authenticator();
    const o = await json<{
      challengeId: string;
      publicKey: { challenge: string };
    }>(
      await passkeys.POST(
        await req('POST', '/api/account/passkeys', {
          as: u,
          body: { action: 'options' },
        }),
      ),
    );
    expect(
      (
        await passkeys.POST(
          await req('POST', '/api/account/passkeys', {
            as: u,
            body: {
              action: 'register',
              challengeId: o.body.challengeId,
              name: 'k',
              credential: a.register(o.body.publicKey.challenge),
            },
          }),
        )
      ).status,
    ).toBe(201);
    const sid = staleSession(u);
    // Adding or removing passkeys from an old session needs a confirmation too.
    expect(
      (
        await json<{ code: string }>(
          await passkeys.POST(
            await req('POST', '/api/account/passkeys', {
              as: u,
              sid,
              body: { action: 'options' },
            }),
          ),
        )
      ).body.code,
    ).toBe('reauth_required');
    const set = await json<{ code: string; methods: string[] }>(
      await users.PATCH(
        await req('PATCH', '/api/users', {
          as: u,
          sid,
          body: { action: 'set_role', userIds: ['usr_x'], role: 'operator' },
        }),
      ),
    );
    expect(set.body).toMatchObject({
      code: 'reauth_required',
      methods: ['passkey'],
    });

    const opts = await confirm(u, sid, { method: 'passkey_options' });
    expect(opts.body.publicKey!.allowCredentials).toHaveLength(1);
    expect(
      (
        await confirm(u, sid, {
          method: 'passkey',
          challengeId: opts.body.challengeId,
          credential: a.assert(opts.body.publicKey!.challenge, { flags: 0x01 }),
        })
      ).status,
    ).toBe(401);
    const other = new Authenticator();
    const o2 = await confirm(u, sid, { method: 'passkey_options' });
    expect(
      (
        await confirm(u, sid, {
          method: 'passkey',
          challengeId: o2.body.challengeId,
          credential: other.assert(o2.body.publicKey!.challenge),
        })
      ).status,
    ).toBe(401);
    const o3 = await confirm(u, sid, { method: 'passkey_options' });
    expect(
      (
        await confirm(u, sid, {
          method: 'passkey',
          challengeId: o3.body.challengeId,
          credential: a.assert(o3.body.publicKey!.challenge),
        })
      ).status,
    ).toBe(200);
    expect(
      (
        await json<{ challengeId: string }>(
          await passkeys.POST(
            await req('POST', '/api/account/passkeys', {
              as: u,
              sid,
              body: { action: 'options' },
            }),
          ),
        )
      ).body.challengeId,
    ).toBeTruthy();
  });

  it('single sign-on accounts are sent to sign in again', async () => {
    const u = makeUser('stepup-sso@x.org', 'security_admin');
    const sid = staleSession(u);
    expect((await putSecurity(u, sid)).body.methods).toEqual(['sso']);
    expect((await confirm(u, sid, { method: 'sso' })).body.error).toMatch(/single sign-on/);
  });
});
