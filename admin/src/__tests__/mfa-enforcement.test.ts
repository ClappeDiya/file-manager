/** Organization-wide two-factor requirement: sessions are limited to enrollment until MFA is on. */
import { describe, expect, it } from 'vitest';
import { hash } from 'bcryptjs';
import { createOrg, setMembership } from '@/lib/db/repos/orgs';
import { getUser } from '@/lib/db/repos/users';
import { mfaRequired, setSecurity } from '@/lib/db/repos/security';
import { listAudit } from '@/lib/db/repos/audit';
import { hotp, totpStep } from '@/lib/totp';
import { cookieFrom, json, makeUser, req } from './helpers';
import * as login from '@/app/api/auth/login/route';
import * as mfa from '@/app/api/account/mfa/route';
import * as session from '@/app/api/auth/session/route';
import * as users from '@/app/api/users/route';
import * as security from '@/app/api/security/route';

const PW = 'Valid-Strong-Pass-8';

describe('MFA enforcement', () => {
  it('limits sessions to enrollment until two-factor sign-in is on', async () => {
    const org = createOrg({ name: 'MFA org' });
    const sec = makeUser('mfa-sec@x.org', 'security_admin', org.id);
    const u = makeUser('mfa-member@x.org', 'org_admin', org.id, { passwordHash: await hash(PW, 4) });
    // Before the rule: normal access.
    expect((await users.GET(await req('GET', '/api/users', { as: u }))).status).toBe(200);
    expect((await security.PUT(await req('PUT', '/api/security', { as: sec, body: { requireMfa: 'yes' } }))).status).toBe(400);
    expect((await security.PUT(await req('PUT', '/api/security', { as: sec, body: { requireMfa: true } }))).status).toBe(200);

    // An existing session is limited at once.
    const blocked = await json<{ code: string }>(await users.GET(await req('GET', '/api/users', { as: u })));
    expect(blocked).toMatchObject({ status: 403, body: { code: 'mfa_setup_required' } });
    expect((await session.GET(await req('GET', '/api/auth/session', { as: u }))).status).toBe(200);
    expect((await mfa.GET(await req('GET', '/api/account/mfa', { as: u }))).status).toBe(200);

    // Sign-in succeeds but points to the setup.
    const signin = await json<{ mfaSetupRequired: boolean; redirect: string }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: PW } })));
    expect(signin.body).toMatchObject({ mfaSetupRequired: true, redirect: '/account?setup-mfa=1' });
    expect(listAudit({ eventType: 'auth.mfa_setup_required' }).total).toBe(1);

    // Enrolling lifts the limit; disabling is refused while required.
    const begin = await json<{ secret: string }>(await mfa.POST(await req('POST', '/api/account/mfa', { as: u, body: { action: 'begin' } })));
    const ok = await mfa.POST(await req('POST', '/api/account/mfa', { as: u, body: { action: 'confirm', code: hotp(begin.body.secret, totpStep()) } }));
    expect(ok.status).toBe(200);
    expect((await users.GET(await req('GET', '/api/users', { as: getUser(u.id)! }))).status).toBe(200);
    const off = await mfa.POST(await req('POST', '/api/account/mfa', { as: getUser(u.id)!, body: { action: 'disable', code: hotp(begin.body.secret, totpStep() + 1) } }));
    expect(off.status).toBe(403);
    // Signing in now asks for the code as usual.
    const again = await json<{ mfaRequired: boolean }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: PW } })));
    expect(again.body.mfaRequired).toBe(true);
    expect(cookieFrom(await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: PW } })))).toBeNull();
  });

  it('applies the strictest organization and can exempt single sign-on', () => {
    const a = createOrg({ name: 'MFA A' });
    const b = createOrg({ name: 'MFA B' });
    const u = makeUser('mfa-multi@x.org', 'user', a.id);
    setMembership(u.id, b.id, 'user');
    expect(mfaRequired(getUser(u.id)!, false)).toBe(false);
    setSecurity(b.id, { requireMfa: true, ssoExemptFromMfa: true }, 't');
    expect(mfaRequired(getUser(u.id)!, false)).toBe(true);
    expect(mfaRequired(getUser(u.id)!, true)).toBe(false);
    setSecurity(a.id, { requireMfa: true, ssoExemptFromMfa: false }, 't');
    expect(mfaRequired(getUser(u.id)!, true)).toBe(true);
  });
});
