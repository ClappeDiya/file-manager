/** Console IP allowlists: CIDR parsing, the client address, enforcement, and no self-lockout. */
import { afterEach, describe, expect, it } from 'vitest';
import { hash } from 'bcryptjs';
import { createOrg, setMembership } from '@/lib/db/repos/orgs';
import { getSecurity, setSecurity } from '@/lib/db/repos/security';
import { listAudit } from '@/lib/db/repos/audit';
import { clientIp, ipAllowed, normalizeAllowlist, parseCidr, parseIp } from '@/lib/netacl';
import { json, makeUser, req } from './helpers';
import * as security from '@/app/api/security/route';
import * as users from '@/app/api/users/route';
import * as logout from '@/app/api/auth/logout/route';
import * as login from '@/app/api/auth/login/route';
import * as orgSwitch from '@/app/api/orgs/switch/route';

const from = (ip: string) => ({ 'x-forwarded-for': ip });

describe('netacl', () => {
  it('parses IPv4, IPv6, mapped addresses and ranges', () => {
    expect([...parseIp('10.1.2.3')!]).toEqual([10, 1, 2, 3]);
    expect(parseIp('::ffff:10.1.2.3')).toEqual(parseIp('10.1.2.3'));
    expect(parseIp('2001:db8::1')!.length).toBe(16);
    expect(parseIp('[::1]')![15]).toBe(1);
    for (const bad of ['256.1.1.1', '1.2.3', 'x', '1::2::3', '1:2:3:4:5:6:7:8:9', '']) expect(parseIp(bad)).toBeNull();
    expect(ipAllowed(['10.0.0.0/8'], '10.200.1.1')).toBe(true);
    expect(ipAllowed(['10.0.0.0/8'], '11.0.0.1')).toBe(false);
    expect(ipAllowed(['192.168.1.128/25'], '192.168.1.127')).toBe(false);
    expect(ipAllowed(['192.168.1.128/25'], '192.168.1.200')).toBe(true);
    expect(ipAllowed(['2001:db8::/32'], '2001:db8:ffff::5')).toBe(true);
    expect(ipAllowed(['2001:db8::/32'], '2001:db9::5')).toBe(false);
    expect(ipAllowed(['10.0.0.0/8'], '::ffff:10.0.0.9')).toBe(true);
    expect(ipAllowed(['203.0.113.7'], '203.0.113.7')).toBe(true);
    expect(ipAllowed(['0.0.0.0/0'], '2001:db8::1')).toBe(false); // families do not mix
    expect(ipAllowed([], 'unknown')).toBe(true);
    expect(ipAllowed(['10.0.0.0/8'], 'unknown')).toBe(false);
    expect(() => parseCidr('10.0.0.0/33')).toThrow(/0 to 32/);
    expect(() => parseCidr('10.0.0.0/8/8')).toThrow();
    expect(normalizeAllowlist([' 10.0.0.0/8 ', '', '10.0.0.0/8'])).toEqual(['10.0.0.0/8']);
    expect(() => normalizeAllowlist(['nope'])).toThrow(/not an IP/);
  });

  it('takes the address the trusted proxy added, not one the client sent', () => {
    const h = (v: Record<string, string>) => ({ headers: new Headers(v) });
    expect(clientIp(h({ 'x-forwarded-for': '6.6.6.6, 198.51.100.4' }))).toBe('198.51.100.4');
    expect(clientIp(h({ 'x-forwarded-for': '198.51.100.4' }))).toBe('198.51.100.4');
    process.env.TRUSTED_PROXY_HOPS = '2';
    expect(clientIp(h({ 'x-forwarded-for': '6.6.6.6, 198.51.100.4, 10.0.0.2' }))).toBe('198.51.100.4');
    expect(clientIp(h({ 'x-real-ip': '192.0.2.1' }))).toBe('192.0.2.1');
    expect(clientIp(h({}))).toBe('unknown');
  });
});

afterEach(() => {
  delete process.env.TRUSTED_PROXY_HOPS;
});

describe('console IP allowlist', () => {
  it('refuses a list that would lock out the saving administrator and validates entries', async () => {
    const org = createOrg({ name: 'Net org' });
    const admin = makeUser('net-admin@x.org', 'security_admin', org.id);
    const put = async (ipAllowlist: unknown, ip = '203.0.113.7') =>
      json<{ error?: string; settings: { ipAllowlist: string[] } }>(
        await security.PUT(
          await req('PUT', '/api/security', {
            as: admin,
            body: { ipAllowlist },
            headers: from(ip),
          }),
        ),
      );
    expect((await put(['198.51.100.0/24'])).body.error).toMatch(/your current address \(203\.0\.113\.7\)/);
    expect((await put(['bogus'])).status).toBe(400);
    expect((await put('10.0.0.0/8')).status).toBe(400);
    const ok = await put(['203.0.113.0/24', ' 198.51.100.0/24 ']);
    expect(ok.status).toBe(200);
    expect(ok.body.settings.ipAllowlist).toEqual(['203.0.113.0/24', '198.51.100.0/24']);
    expect(getSecurity(org.id).ipAllowlist).toHaveLength(2);
  });

  it('only admits sessions from allowed networks; signing out still works', async () => {
    const org = createOrg({ name: 'Locked org' });
    const admin = makeUser('locked@x.org', 'org_admin', org.id);
    setSecurity(org.id, { ipAllowlist: ['203.0.113.0/24'] }, 't');
    expect(
      (
        await users.GET(
          await req('GET', '/api/users', {
            as: admin,
            headers: from('203.0.113.50'),
          }),
        )
      ).status,
    ).toBe(200);
    const blocked = await json<{ code: string }>(
      await users.GET(
        await req('GET', '/api/users', {
          as: admin,
          headers: from('192.0.2.9'),
        }),
      ),
    );
    expect(blocked.status).toBe(403);
    expect(blocked.body.code).toBe('ip_not_allowed');
    // A spoofed client header in front of the proxy's entry does not help.
    expect(
      (
        await users.GET(
          await req('GET', '/api/users', {
            as: admin,
            headers: from('203.0.113.50, 192.0.2.9'),
          }),
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await logout.POST(
          await req('POST', '/api/auth/logout', {
            as: admin,
            headers: from('192.0.2.9'),
          }),
        )
      ).status,
    ).toBe(200);
  });

  it('signs in to an organization that admits the address, or refuses and audits', async () => {
    const home = createOrg({ name: 'Home net' });
    const other = createOrg({ name: 'Other net' });
    const u = makeUser('multi-net@x.org', 'operator', home.id, {
      passwordHash: await hash('Net-Pass-123456', 4),
    });
    setMembership(u.id, other.id, 'operator');
    setSecurity(home.id, { ipAllowlist: ['10.0.0.0/8'] }, 't');
    const signIn = async (ip: string) =>
      json<{ orgId?: string; code?: string }>(
        await login.POST(
          await req('POST', '/api/auth/login', {
            body: { email: u.email, password: 'Net-Pass-123456' },
            headers: from(ip),
          }),
        ),
      );
    expect((await signIn('10.1.1.1')).body.orgId).toBe(home.id);
    expect((await signIn('192.0.2.30')).body.orgId).toBe(other.id);
    setSecurity(other.id, { ipAllowlist: ['10.0.0.0/8'] }, 't');
    const refused = await signIn('192.0.2.31');
    expect(refused.status).toBe(403);
    expect(refused.body.code).toBe('ip_not_allowed');
    expect(listAudit({ eventType: 'auth.login_failed' }).entries[0].description).toContain('192.0.2.31');
    // Switching into an organization that refuses the address is refused.
    setSecurity(other.id, { ipAllowlist: [] }, 't');
    const sw = await orgSwitch.POST(
      await req('POST', '/api/orgs/switch', {
        as: u,
        org: other.id,
        body: { orgId: home.id },
        headers: from('192.0.2.32'),
      }),
    );
    expect(sw.status).toBe(403);
  });
});
