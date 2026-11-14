/**
 * LDAP / AD sign-in against a real (in-process) directory: service bind,
 * user search with escaped input, user bind, first-login accounts with
 * mapped groups, refusals, and configuration checks.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import ldap from 'ldapjs';
import type { AddressInfo } from 'node:net';
import { createProvider, listSsoAttempts } from '@/lib/db/repos/sso';
import { createGroup, groupMembers } from '@/lib/db/repos/groups';
import { createOrg } from '@/lib/db/repos/orgs';
import { findUserByEmail } from '@/lib/db/repos/users';
import { escapeFilter, groupName } from '@/lib/ldap';
import { startUrl } from '@/lib/sso';
import { validateLdapConfig } from '@/lib/ldap-config';
import { json, req } from './helpers';
import * as ldapLogin from '@/app/api/auth/ldap/route';

const BASE = 'dc=example,dc=com';
const SVC = 'cn=svc,dc=example,dc=com';
type Entry = { dn: string; password: string; attributes: Record<string, string | string[]> };
const ENTRIES: Entry[] = [
  { dn: `uid=alice,ou=people,${BASE}`, password: 'alice-pw', attributes: { uid: 'alice', mail: 'Alice@Example.com', displayName: 'Alice Archer', memberOf: [`cn=Ops,ou=groups,${BASE}`, `cn=Everyone,ou=groups,${BASE}`], entryUUID: '7f2c-alice', objectClass: 'person' } },
  { dn: `uid=bob,ou=people,${BASE}`, password: 'bob-pw', attributes: { uid: 'bob', displayName: 'Bob', objectClass: 'person' } },
  { dn: `uid=dup,ou=a,${BASE}`, password: 'x', attributes: { uid: 'dup', objectClass: 'person' } },
  { dn: `uid=dup,ou=b,${BASE}`, password: 'x', attributes: { uid: 'dup', objectClass: 'person' } },
];
// ldapjs v3's server, typed by what the test uses (its published types predate v3).
type Named = { toString(): string };
type Req = { dn: Named; credentials?: unknown; filter: Named; connection: { ldap: { bindDN: Named } } };
type Res = { end(): void; send(entry: unknown): void; attributes: string[] };
type Next = (err?: unknown) => void;
interface DirectoryServer {
  bind(base: string, handler: (r: Req, res: Res, next: Next) => void): void;
  search(base: string, handler: (r: Req, res: Res, next: Next) => void): void;
  listen(port: number, host: string, cb: () => void): void;
  address(): AddressInfo;
  close(): void;
}
const searches: string[] = [];
let server: DirectoryServer;
let url = '';

beforeAll(async () => {
  server = ldap.createServer() as unknown as DirectoryServer;
  server.bind(BASE, (r, res, next) => {
    const dn = r.dn.toString().replace(/\s/g, '');
    const pw = String(r.credentials);
    if (dn === SVC && pw === 'svc-pw') return (res.end(), next());
    const e = ENTRIES.find((x) => x.dn === dn);
    if (!e || !pw || e.password !== pw) return next(new ldap.InvalidCredentialsError());
    res.end();
    next();
  });
  server.search(BASE, (r, res, next) => {
    if (r.connection.ldap.bindDN.toString().replace(/\s/g, '') !== SVC) return next(new ldap.InsufficientAccessRightsError());
    searches.push(r.filter.toString());
    // Like real directories: attribute names and these values match ignoring case.
    const filter = ldap.parseFilter(r.filter.toString().toLowerCase());
    const lower = (a: Entry['attributes']) => Object.fromEntries(Object.entries(a).map(([k, v]) => [k.toLowerCase(), Array.isArray(v) ? v.map((x) => x.toLowerCase()) : v.toLowerCase()]));
    // ldapjs compares requested attribute names case-sensitively against lowercased ones; directories don't.
    res.attributes = res.attributes.map((x) => x.toLowerCase());
    for (const e of ENTRIES) if (filter.matches(lower(e.attributes))) res.send({ dn: e.dn, attributes: e.attributes });
    res.end();
    next();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()));
  url = `ldap://127.0.0.1:${server.address().port}`;
});

afterAll(() => server.close());

const login = async (providerId: string, username: string, password: string) =>
  json<{ error?: string; ok?: boolean; redirect?: string }>(await ldapLogin.POST(await req('POST', '/api/auth/ldap', { body: { providerId, username, password } })));

describe('LDAP sign-in', () => {
  it('signs directory users in, creating accounts with mapped groups', async () => {
    const org = createOrg({ name: 'Directory org' });
    const ops = createGroup(org.id, { name: 'Operations' });
    const p = createProvider(org.id, {
      name: 'Corp AD',
      kind: 'ldap',
      config: { url, bindDn: SVC, baseDn: BASE, userFilter: '(&(objectClass=person)(uid={username}))' },
      secret: 'svc-pw',
      groupMapping: { Ops: ops.id },
      domains: ['example.com'],
      allowJit: true,
    });
    const ok = await login(p.id, 'Alice', 'alice-pw');
    expect(ok.status).toBe(200);
    const alice = findUserByEmail('alice@example.com')!;
    expect(alice.name).toBe('Alice Archer');
    expect(groupMembers(ops.id).map((m) => m.userId)).toContain(alice.id);
    // The same directory entry signs in to the same account.
    expect((await login(p.id, 'alice', 'alice-pw')).status).toBe(200);

    expect((await login(p.id, 'alice', 'wrong')).body.error).toBe('Unknown user name or wrong password');
    expect((await login(p.id, 'alice', '')).body.error).toContain('Enter your directory user name and password');
    // Filter metacharacters are escaped, so "*" matches nobody instead of everybody.
    expect((await login(p.id, '*', 'x')).body.error).toBe('Unknown user name or wrong password');
    expect(searches.some((f) => f.includes('uid=\\2a'))).toBe(true);
    expect((await login(p.id, 'dup', 'x')).body.error).toContain('More than one directory entry');
    // Entries without an email address can't become accounts.
    expect((await login(p.id, 'bob', 'bob-pw')).body.error).toContain('did not send an email address');

    // It is not a redirect flow.
    await expect(startUrl(p, 'http://localhost:3001', '/', null)).rejects.toThrow('directory user name');
    const attempts = listSsoAttempts(org.id, p.id);
    expect(attempts.some((a) => a.ok && a.steps.some((s) => s.step === 'user_bind'))).toBe(true);
    expect(attempts.some((a) => !a.ok && a.stage === 'user_bind')).toBe(true);
  });

  it('reports a wrong service account and refuses unsafe configurations', async () => {
    const org = createOrg({ name: 'Directory org 2' });
    const p = createProvider(org.id, { name: 'Bad svc', kind: 'ldap', config: { url, bindDn: SVC, baseDn: BASE, userFilter: '(uid={username})' }, secret: 'nope', allowJit: true });
    expect((await login(p.id, 'alice', 'alice-pw')).body.error).toContain('could not sign in to the directory');
    expect(() => createProvider(org.id, { name: 'No secret', kind: 'ldap', config: { url, bindDn: SVC, baseDn: BASE, userFilter: '(uid={username})' } })).toThrow('service account password');

    const base = { bindDn: SVC, baseDn: BASE, userFilter: '(uid={username})' };
    expect(validateLdapConfig({ ...base, url: 'ldap://dc.corp.example.com' })).toContain('unencrypted');
    expect(validateLdapConfig({ ...base, url: 'ldap://dc.corp.example.com', startTls: true })).toBeNull();
    expect(validateLdapConfig({ ...base, url: 'ldaps://dc.corp.example.com:636' })).toBeNull();
    expect(validateLdapConfig({ ...base, url: 'https://dc' })).toContain('ldaps://');
    expect(validateLdapConfig({ ...base, url: 'ldaps://dc', userFilter: '(uid=x)' })).toContain('{username}');
    expect(escapeFilter('a*b(c)\\d\0')).toBe('a\\2ab\\28c\\29\\5cd\\00');
    expect(groupName('CN=Domain Admins,OU=Groups,DC=corp')).toBe('Domain Admins');
    expect(groupName('cn=a\\,b,ou=x')).toBe('a,b');
  });
});
