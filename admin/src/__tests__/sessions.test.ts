/** Server-side sessions: revocation, idle timeout, sign-out everywhere, password changes. */
import { describe, expect, it } from 'vitest';
import { NextRequest } from 'next/server';
import { hash } from 'bcryptjs';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { setSecurity } from '@/lib/db/repos/security';
import { activeSessions } from '@/lib/db/repos/sessions';
import { createPasswordToken } from '@/lib/db/repos/security';
import { SESSION_COOKIE, signSession } from '@/lib/session';
import { cookieFrom, json, makeUser, req } from './helpers';
import * as session from '@/app/api/auth/session/route';
import * as logout from '@/app/api/auth/logout/route';
import * as login from '@/app/api/auth/login/route';
import * as mySessions from '@/app/api/account/sessions/route';
import * as password from '@/app/api/account/password/route';
import * as reset from '@/app/api/auth/password/reset/route';
import * as users from '@/app/api/users/route';
import * as orgSwitch from '@/app/api/orgs/switch/route';

const withCookie = (method: string, url: string, cookie: string, body?: unknown) =>
  new NextRequest(new URL(url, 'http://localhost:3001'), { method, headers: { cookie: `${SESSION_COOKIE}=${cookie}`, 'content-type': 'application/json', origin: 'http://localhost:3001', host: 'localhost:3001' }, body: body === undefined ? undefined : JSON.stringify(body) });

describe('sessions', () => {
  it('signs in to a revocable session and signs out', async () => {
    const u = makeUser('ses@x.org', 'org_admin', 'org_default', { passwordHash: await hash('Session-Pass-123', 4) });
    const r = await login.POST(await req('POST', '/api/auth/login', { body: { email: u.email, password: 'Session-Pass-123' }, headers: { 'user-agent': 'Mozilla/5.0 Firefox/131.0' } }));
    const c = cookieFrom(r)!;
    expect((await session.GET(withCookie('GET', '/api/auth/session', c))).status).toBe(200);
    const mine = await json<{ sessions: { current: boolean; userAgent: string; method: string }[] }>(await mySessions.GET(withCookie('GET', '/api/account/sessions', c)));
    expect(mine.body.sessions).toHaveLength(1);
    expect(mine.body.sessions[0]).toMatchObject({ current: true, method: 'pwd' });
    expect(mine.body.sessions[0].userAgent).toContain('Firefox');
    expect((await logout.POST(withCookie('POST', '/api/auth/logout', c))).status).toBe(200);
    const after = await json<{ error: string }>(await session.GET(withCookie('GET', '/api/auth/session', c)));
    expect(after.status).toBe(401);
    expect(after.body.error).toMatch(/session has ended/);
  });

  it('rejects tokens without a server-side session (cookies from before this change)', async () => {
    const u = makeUser('ses-old@x.org', 'org_admin');
    const { SignJWT } = await import('jose');
    const legacy = await new SignJWT({ uid: u.id, org: 'org_default', amr: ['pwd'] }).setProtectedHeader({ alg: 'HS256' }).setExpirationTime('1h').sign(new TextEncoder().encode(process.env.NEXTAUTH_SECRET!));
    expect((await session.GET(withCookie('GET', '/api/auth/session', legacy))).status).toBe(401);
  });

  it('signs out idle sessions by the strictest organization', async () => {
    const org = createOrg({ name: 'Idle org' });
    const u = makeUser('ses-idle@x.org', 'org_admin', org.id);
    const c = await signSession(u, org.id, ['pwd']);
    expect((await session.GET(withCookie('GET', '/api/auth/session', c))).status).toBe(200);
    setSecurity(org.id, { sessionIdleMinutes: 15 }, 't');
    db().prepare('UPDATE sessions SET last_seen_at = ? WHERE user_id = ?').run(new Date(Date.now() - 16 * 60_000).toISOString(), u.id);
    const r = await json<{ error: string }>(await session.GET(withCookie('GET', '/api/auth/session', c)));
    expect(r.status).toBe(401);
    expect(r.body.error).toMatch(/inactivity/);
    expect(() => setSecurity(org.id, { sessionIdleMinutes: 2 }, 't')).toThrow(/5 to 480/);
  });

  it('keeps the session across organization switches and ends other sessions on demand', async () => {
    const other = createOrg({ name: 'Switch target' });
    const u = makeUser('ses-multi@x.org', 'org_admin');
    const { setMembership } = await import('@/lib/db/repos/orgs');
    setMembership(u.id, other.id, 'org_admin');
    const c1 = await signSession(u, 'org_default', ['pwd']);
    const c2 = await signSession(u, 'org_default', ['pwd']);
    const switched = await orgSwitch.POST(withCookie('POST', '/api/orgs/switch', c1, { orgId: other.id }));
    expect(switched.status).toBe(200);
    expect(activeSessions(u.id)).toHaveLength(2); // same session, new organization claim
    const c1b = cookieFrom(switched)!;
    expect((await mySessions.DELETE(withCookie('DELETE', '/api/account/sessions?others=1', c1b))).status).toBe(200);
    expect((await session.GET(withCookie('GET', '/api/auth/session', c2))).status).toBe(401);
    expect((await session.GET(withCookie('GET', '/api/auth/session', c1b))).status).toBe(200);
    expect((await mySessions.DELETE(withCookie('DELETE', '/api/account/sessions?id=ses_nope', c1b))).status).toBe(404);
  });

  it('password changes and resets end other sessions; admins can sign members out', async () => {
    const org = createOrg({ name: 'Ses admin' });
    const admin = makeUser('ses-adm@x.org', 'org_admin', org.id);
    const u = makeUser('ses-member@x.org', 'operator', org.id, { passwordHash: await hash('Member-Pass-456', 4) });
    const a = await signSession(u, org.id, ['pwd']);
    const b = await signSession(u, org.id, ['pwd']);
    const ch = await json<{ otherSessionsSignedOut: number }>(await password.POST(withCookie('POST', '/api/account/password', a, { current: 'Member-Pass-456', password: 'Changed-Pass-789' })));
    expect(ch.body.otherSessionsSignedOut).toBe(1);
    expect((await session.GET(withCookie('GET', '/api/auth/session', b))).status).toBe(401);
    expect((await session.GET(withCookie('GET', '/api/auth/session', a))).status).toBe(200);

    const token = createPasswordToken(u.id, 'reset');
    expect((await reset.POST(await req('POST', '/api/auth/password/reset', { body: { token, password: 'Reset-Pass-1011' } }))).status).toBe(200);
    expect((await session.GET(withCookie('GET', '/api/auth/session', a))).status).toBe(401);

    const c = await signSession(u, org.id, ['pwd']);
    expect((await users.PATCH(await req('PATCH', '/api/users', { as: admin, org: org.id, body: { action: 'sign_out', userIds: [u.id] } }))).status).toBe(200);
    expect((await session.GET(withCookie('GET', '/api/auth/session', c))).status).toBe(401);
    await signSession(u, org.id, ["pwd"]);
    await users.PATCH(await req('PATCH', '/api/users', { as: admin, org: org.id, body: { action: 'deactivate', userIds: [u.id] } }));
    expect(activeSessions(u.id)).toHaveLength(0);
  });
});
