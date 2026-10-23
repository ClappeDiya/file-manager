/**
 * Passkeys (WebAuthn) against a software authenticator: registration,
 * sign-in, and the checks that stop wrong origins, other sites, replayed
 * challenges, forged signatures and cloned keys. Also: CBOR decoding and
 * passkeys satisfying a two-factor requirement.
 */
import { describe, expect, it } from 'vitest';
import { createOrg } from '@/lib/db/repos/orgs';
import { setSecurity } from '@/lib/db/repos/security';
import { getPasskey } from '@/lib/db/repos/passkeys';
import { listAudit } from '@/lib/db/repos/audit';
import { decodeCbor } from '@/lib/cbor';
import { cookieFrom, json, makeUser, req } from './helpers';
import { Authenticator, b64, cbor } from './webauthn-helpers';
import * as passkeys from '@/app/api/account/passkeys/route';
import * as options from '@/app/api/auth/passkey/options/route';
import * as verify from '@/app/api/auth/passkey/verify/route';
import * as users from '@/app/api/users/route';

async function register(u: ReturnType<typeof makeUser>, a: Authenticator, name = 'Key', tweak: Parameters<Authenticator['register']>[1] = {}) {
  const o = await json<{ challengeId: string; publicKey: { challenge: string; rp: { id: string } } }>(await passkeys.POST(await req('POST', '/api/account/passkeys', { as: u, body: { action: 'options' } })));
  return json<{ passkey: { credentialId: string }; error?: string }>(await passkeys.POST(await req('POST', '/api/account/passkeys', { as: u, body: { action: 'register', challengeId: o.body.challengeId, name, credential: a.register(o.body.publicKey.challenge, tweak) } })));
}

async function signIn(a: Authenticator, tweak: Parameters<Authenticator['assert']>[1] = {}, reuse?: { challengeId: string; challenge: string }) {
  const o = reuse ?? (await json<{ challengeId: string; publicKey: { challenge: string } }>(await options.POST(await req('POST', '/api/auth/passkey/options')))).body;
  const challengeId = 'challengeId' in o ? o.challengeId : '';
  const challenge = 'publicKey' in o ? (o as { publicKey: { challenge: string } }).publicKey.challenge : (o as { challenge: string }).challenge;
  // A fresh client address per attempt: the deliberate failures must not trip the login rate limit.
  const ip = `198.18.${Math.floor(Math.random() * 250)}.${Math.floor(Math.random() * 250)}`;
  const res = await verify.POST(await req('POST', '/api/auth/passkey/verify', { body: { challengeId, credential: a.assert(challenge, tweak) }, headers: { 'x-forwarded-for': ip } }));
  return { res, challengeId, challenge };
}

describe('CBOR', () => {
  it('decodes maps, arrays, strings, integers and rejects junk', () => {
    const v = decodeCbor(new Uint8Array(cbor(new Map<unknown, unknown>([['a', 1], [-2, Buffer.from([1, 2])], [3, 'x']]))));
    expect(v.value).toEqual(new Map<unknown, unknown>([['a', 1], [-2, new Uint8Array([1, 2])], [3, 'x']]));
    expect(decodeCbor(new Uint8Array([0x82, 0xf5, 0xf6])).value).toEqual([true, null]);
    expect(() => decodeCbor(new Uint8Array([0x62, 0x61]))).toThrow(/Truncated/);
    expect(() => decodeCbor(new Uint8Array([0x01, 0x02]))).toThrow(/Trailing/);
    expect(() => decodeCbor(new Uint8Array([0x9f]))).toThrow(/Unsupported/);
  });
});

describe('passkeys', () => {
  it('registers and signs in with ES256 and Ed25519 passkeys', async () => {
    const org = createOrg({ name: 'Passkey org' });
    const u = makeUser('pk@x.org', 'org_admin', org.id);
    const es = new Authenticator('es256');
    const ed = new Authenticator('ed25519');
    expect((await register(u, es, 'Laptop')).status).toBe(201);
    expect((await register(u, ed, 'Security key')).status).toBe(201);
    const list = await (await passkeys.GET(await req('GET', '/api/account/passkeys', { as: u }))).text();
    expect(JSON.parse(list).passkeys.map((p: { name: string }) => p.name)).toEqual(['Laptop', 'Security key']);
    expect(list).not.toContain('BEGIN PUBLIC KEY');

    const { res } = await signIn(es);
    expect(res.status).toBe(200);
    expect(cookieFrom(res)).toBeTruthy();
    const audit = listAudit({ eventType: 'auth.login' }).entries[0];
    expect(audit.description).toContain('passkey + mfa');
    expect(getPasskey(b64(es.id))!.signCount).toBe(1);
    expect((await signIn(ed)).res.status).toBe(200);
    // Without user verification it is one factor: still a valid sign-in for an account without TOTP.
    const up = await signIn(es, { flags: 0x01 });
    expect(up.res.status).toBe(200);
    expect(listAudit({ eventType: 'auth.login' }).entries[0].description).toContain('(passkey)');
  });

  it('refuses wrong origins, other sites, replays, forged signatures and cloned keys', async () => {
    const u = makeUser('pk-checks@x.org', 'user');
    const a = new Authenticator();
    expect((await register(u, a, 'k', { origin: 'https://evil.example' })).body.error).toMatch(/origin/);
    expect((await register(u, a, 'k', { rpId: 'evil.example' })).body.error).toMatch(/another site/);
    expect((await register(u, a, 'k', { flags: 0x40 })).body.error).toMatch(/presence/);
    expect((await register(u, a)).status).toBe(201);
    expect((await register(u, a)).status).toBe(409); // same credential twice

    expect((await signIn(a, { origin: 'https://evil.example' })).res.status).toBe(401);
    expect((await signIn(a, { rpId: 'evil.example' })).res.status).toBe(401);
    expect((await signIn(a, { tamper: true })).res.status).toBe(401);
    const ok = await signIn(a);
    expect(ok.res.status).toBe(200);
    // The same challenge cannot be used twice.
    expect((await signIn(a, {}, { challengeId: ok.challengeId, challenge: ok.challenge })).res.status).toBe(401);
    // A counter that does not go up means a copied key.
    expect((await signIn(a, { counter: getPasskey(b64(a.id))!.signCount })).res.status).toBe(401);
    const unknown = new Authenticator();
    expect((await signIn(unknown)).res.status).toBe(401);
    expect(listAudit({ eventType: 'auth.login_failed' }).entries.some((e) => e.description.includes('counter went backwards'))).toBe(true);
  });

  it('satisfies a two-factor requirement and keeps the last factor while required', async () => {
    const org = createOrg({ name: 'Passkey MFA' });
    const u = makeUser('pk-mfa@x.org', 'operator', org.id);
    setSecurity(org.id, { requireMfa: true }, 't');
    expect((await users.GET(await req('GET', '/api/users', { as: u }))).status).toBe(403);
    const a = new Authenticator();
    const reg = await register(u, a);
    expect(reg.status).toBe(201);
    expect((await users.GET(await req('GET', '/api/users', { as: u }))).status).toBe(200);
    const id = reg.body.passkey.credentialId;
    expect((await passkeys.DELETE(await req('DELETE', `/api/account/passkeys?id=${id}`, { as: u }))).status).toBe(403);
    const b = new Authenticator();
    await register(u, b);
    expect((await passkeys.DELETE(await req('DELETE', `/api/account/passkeys?id=${id}`, { as: u }))).status).toBe(200);
    expect((await passkeys.PATCH(await req('PATCH', '/api/account/passkeys', { as: u, body: { id: b64(b.id), name: 'Renamed' } }))).status).toBe(200);
    const other = makeUser('pk-other@x.org', 'user');
    expect((await passkeys.DELETE(await req('DELETE', `/api/account/passkeys?id=${b64(b.id)}`, { as: other }))).status).toBe(404);
  });
});
