/**
 * Password strength, organization password age (enforced at sign-in),
 * self-service reset by e-mailed link, and changing one's own password.
 */
import { describe, expect, it } from 'vitest';
import { hash, compare } from 'bcryptjs';
import { db } from '@/lib/db';
import { createOrg, setMembership } from '@/lib/db/repos/orgs';
import { getUser, updateUser } from '@/lib/db/repos/users';
import { createPasswordToken, getSecurity, passwordExpired } from '@/lib/db/repos/security';
import { createChannel } from '@/lib/db/repos/channels';
import { listOutbox } from '@/lib/db/repos/outbox';
import { createInvitation } from '@/lib/db/repos/invitations';
import { listAudit } from '@/lib/db/repos/audit';
import { checkPassword } from '@/lib/password-policy';
import { cookieFrom, json, makeUser, req } from './helpers';
import * as login from '@/app/api/auth/login/route';
import * as forgot from '@/app/api/auth/password/forgot/route';
import * as reset from '@/app/api/auth/password/reset/route';
import * as change from '@/app/api/account/password/route';
import * as security from '@/app/api/security/route';
import * as accept from '@/app/api/invitations/accept/route';

const GOOD = 'Correct-Horse-7-Battery';

describe('password policy', () => {
  it('scores and explains', () => {
    expect(checkPassword('short1A!').problems[0]).toMatch(/at least 12/);
    expect(checkPassword('alllowercaseletters12').ok).toBe(true); // 16+ passphrase-length
    expect(checkPassword('alllowercase1').problems.join()).toMatch(/Mix upper case/);
    expect(checkPassword('Password!!!!!!1').problems.join()).toMatch(/repeating|commonly/);
    expect(checkPassword('Abcdefgh-12345').problems.join()).toMatch(/sequences/);
    expect(checkPassword('Ana-Lima-2026-x', { name: 'Ana Lima' }).problems.join()).toMatch(/name or e-mail/);
    expect(checkPassword('Xjordan-77-Q!', { email: 'jordan@x.org' }).problems.join()).toMatch(/name or e-mail/);
    expect(checkPassword('Qwerty!!Qwerty1').ok).toBe(true);
    const strong = checkPassword(GOOD);
    expect(strong).toMatchObject({ ok: true });
    expect(strong.score).toBeGreaterThanOrEqual(3);
    expect(checkPassword('x').score).toBeLessThanOrEqual(1);
  });
});

describe('password lifecycle', () => {
  it('expires passwords by the strictest organization and forces a change at sign-in', async () => {
    const a = createOrg({ name: 'Pw A' });
    const b = createOrg({ name: 'Pw B' });
    const admin = makeUser('pw-admin@x.org', 'org_admin', a.id);
    const sec = makeUser('pw-sec@x.org', 'security_admin', a.id);
    expect((await security.PUT(await req('PUT', '/api/security', { as: admin, body: { passwordMaxAgeDays: 30 } }))).status).toBe(403);
    expect((await security.PUT(await req('PUT', '/api/security', { as: sec, body: { passwordMaxAgeDays: -1 } }))).status).toBe(400);
    expect((await json<{ settings: { passwordMaxAgeDays: number } }>(await security.PUT(await req('PUT', '/api/security', { as: sec, body: { passwordMaxAgeDays: 90 } })))).body.settings.passwordMaxAgeDays).toBe(90);
    expect(getSecurity(b.id).passwordMaxAgeDays).toBe(0);

    const u = makeUser('pw-user@x.org', 'user', a.id, { passwordHash: await hash(GOOD, 4) });
    setMembership(u.id, b.id, 'user');
    expect(passwordExpired(getUser(u.id)!)).toBe(false);
    updateUser(u.id, { passwordChangedAt: new Date(Date.now() - 91 * 86_400_000).toISOString() });
    expect(passwordExpired(getUser(u.id)!)).toBe(true);

    const r = await json<{ code: string; changeToken: string }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: GOOD } })));
    expect(r.status).toBe(403);
    expect(r.body.code).toBe('password_expired');
    // A wrong password never reveals the expiry.
    expect((await json<{ code?: string }>(await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: 'wrong-Password-1' } })))).body.code).toBeUndefined();

    const info = await json<{ email: string; purpose: string }>(await reset.GET(await req('GET', `/api/auth/password/reset?token=${r.body.changeToken}`)));
    expect(info.body).toEqual({ email: u.email, purpose: 'expired' });
    // A weak password is refused without using up the token.
    expect((await reset.POST(await req('POST', '/api/auth/password/reset', { body: { token: r.body.changeToken, password: 'weak' } }))).status).toBe(400);
    const done = await reset.POST(await req('POST', '/api/auth/password/reset', { body: { token: r.body.changeToken, password: 'New-Valid-Pass-42' } }));
    expect(done.status).toBe(200);
    expect(cookieFrom(done)).toBeTruthy();
    expect(passwordExpired(getUser(u.id)!)).toBe(false);
    expect((await reset.POST(await req('POST', '/api/auth/password/reset', { body: { token: r.body.changeToken, password: 'Another-Pass-77' } }))).status).toBe(404);
    expect((await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: 'New-Valid-Pass-42' } }))).status).toBe(200);
    expect(listAudit({ eventType: 'auth.password_expired' }).total).toBe(1);
    expect(listAudit({ eventType: 'auth.password_change' }).total).toBeGreaterThan(0);
  });

  it('e-mails reset links without revealing accounts', async () => {
    const org = createOrg({ name: 'Pw reset' });
    createChannel(org.id, { name: 'mail', kind: 'email', config: { host: '127.0.0.1', port: 9, security: 'none', username: '', from: 'ufop@x.org', to: [], useForInvitations: true } });
    const u = makeUser('reset-me@x.org', 'user', org.id, { passwordHash: await hash(GOOD, 4) });
    const sso = makeUser('sso-only@x.org', 'user', org.id);
    const ask = async (email: string) => json<{ ok: boolean; message: string }>(await forgot.POST(await req('POST', '/api/auth/password/forgot', { body: { email } })));
    const known = await ask(u.email);
    const unknown = await ask('nobody@x.org');
    const ssoOnly = await ask(sso.email);
    expect(known.body).toEqual(unknown.body);
    expect(ssoOnly.body).toEqual(unknown.body);
    expect(db().prepare('SELECT COUNT(*) AS n FROM password_resets WHERE user_id = ?').get(u.id)).toEqual({ n: 1 });
    expect(db().prepare('SELECT COUNT(*) AS n FROM password_resets WHERE user_id = ?').get(sso.id)).toEqual({ n: 0 });
    const mail = listOutbox(org.id).find((m) => m.context === 'password_reset')!;
    expect(mail.message.to).toEqual([u.email]);
    expect(mail.message.text).toMatch(/\/reset-password\?token=ufop_pwd_/);

    // Only the newest link works; links expire.
    const t1 = createPasswordToken(u.id, 'reset');
    const t2 = createPasswordToken(u.id, 'reset');
    expect((await reset.GET(await req('GET', `/api/auth/password/reset?token=${t1}`))).status).toBe(404);
    db().prepare('UPDATE password_resets SET expires_at = ? WHERE used_at IS NULL').run(new Date(Date.now() - 1000).toISOString());
    expect((await reset.POST(await req('POST', '/api/auth/password/reset', { body: { token: t2, password: 'Fresh-Valid-Pass-9' } }))).status).toBe(404);
    const t3 = createPasswordToken(u.id, 'reset');
    expect((await reset.POST(await req('POST', '/api/auth/password/reset', { body: { token: t3, password: 'Fresh-Valid-Pass-9' } }))).status).toBe(200);
    expect(await compare('Fresh-Valid-Pass-9', getUser(u.id)!.passwordHash!)).toBe(true);

    // Rate limiting: repeated requests for one address are refused.
    let last = 200;
    for (let i = 0; i < 6; i++) last = (await forgot.POST(await req('POST', '/api/auth/password/forgot', { body: { email: 'flood@x.org' }, headers: { 'x-forwarded-for': `10.9.0.${i}` } }))).status;
    expect(last).toBe(429);
  });

  it('changes the own password and applies the policy to invitations', async () => {
    const org = createOrg({ name: 'Pw change' });
    const u = makeUser('changer@x.org', 'org_admin', org.id, { passwordHash: await hash(GOOD, 4) });
    const post = async (body: unknown) => json<{ error?: string }>(await change.POST(await req('POST', '/api/account/password', { as: u, body })));
    expect((await post({ current: 'wrong', password: 'Other-Valid-Pass-1' })).status).toBe(403);
    expect((await post({ current: GOOD, password: 'weak' })).status).toBe(400);
    expect((await post({ current: GOOD, password: GOOD })).body.error).toMatch(/different/);
    expect((await post({ current: GOOD, password: 'Other-Valid-Pass-1' })).status).toBe(200);
    expect(await compare('Other-Valid-Pass-1', getUser(u.id)!.passwordHash!)).toBe(true);

    const { token } = createInvitation(org.id, { email: 'newbie@x.org', role: 'user' }, u.email);
    const weak = await json<{ problems: string[] }>(await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, name: 'New Bie', password: 'newbie-password' } })));
    expect(weak.status).toBe(400);
    expect(weak.body.problems.length).toBeGreaterThan(0);
    expect((await accept.POST(await req('POST', '/api/invitations/accept', { body: { token, name: 'New Bie', password: 'Strong-Enough-Pass-3' } }))).status).toBe(200);
  });
});
