/**
 * LDAP / Active Directory sign-in: the console finds the user with a
 * service account (search under `baseDn` with `userFilter`), then binds as
 * that entry with the password they typed. The entry's email, name, groups
 * (`memberOf`) and a stable id feed the same account rules as OIDC and SAML
 * (`resolveUser`: linked identity, verified-domain linking, first-login
 * accounts with mapped groups).
 *
 * Connections use LDAPS or StartTLS (plain LDAP only to a loopback
 * address); user input is escaped in filters (RFC 4515), and an empty
 * password is refused before it can become an anonymous bind.
 */
import { Client, type Entry } from 'ldapts';
import { providerSecret, type SsoProvider } from './db/repos/sso';
import type { LdapConfig } from './ldap-config';
import { OrgError } from './db/repos/orgs';
import type { IdpIdentity, SsoTrace } from './sso';

const TIMEOUT_MS = 10_000;

/** RFC 4515 filter value escaping. */
export function escapeFilter(value: string): string {
  return value.replace(/[\\*()\0]/g, (c) => `\\${c.charCodeAt(0).toString(16).padStart(2, '0')}`);
}

const text = (v: Entry[string] | undefined): string => {
  const first = Array.isArray(v) ? v[0] : v;
  return first === undefined ? '' : Buffer.isBuffer(first) ? first.toString('utf8') : String(first);
};
const list = (v: Entry[string] | undefined): string[] => (v === undefined ? [] : (Array.isArray(v) ? v : [v]).map((x) => (Buffer.isBuffer(x) ? x.toString('utf8') : String(x))));

/** An attribute of `entry`, whatever case the server spelled its name in. */
function attr(entry: Entry, name: string): Entry[string] | undefined {
  const key = Object.keys(entry).find((k) => k.toLowerCase() === name.toLowerCase());
  return key === undefined ? undefined : entry[key];
}

/** `CN=Admins,OU=Groups,DC=x` → `Admins`. */
export function groupName(dn: string): string {
  const m = /^\s*cn=((?:\\.|[^,])+)/i.exec(dn);
  return m ? m[1].replace(/\\(.)/g, '$1') : dn;
}

/** The person behind `username`/`password`, verified by the directory. */
export async function ldapIdentity(p: SsoProvider, username: string, password: string, trace: SsoTrace): Promise<IdpIdentity> {
  const c = p.config as unknown as LdapConfig;
  const user = username.trim();
  trace.email = user;
  if (!user || !password) throw trace.fail('credentials', 'Enter your directory user name and password');
  const tlsOptions = c.caCert ? { ca: [c.caCert] } : undefined;
  const client = new Client({ url: c.url, timeout: TIMEOUT_MS, connectTimeout: TIMEOUT_MS, tlsOptions });
  try {
    if (c.startTls) await client.startTLS(tlsOptions ?? {});
    try {
      await client.bind(c.bindDn, providerSecret(p.id) ?? '');
    } catch (e) {
      throw trace.fail('service_bind', `The console could not sign in to the directory: ${(e as Error).message}`);
    }
    trace.ok('service_bind');
    const emailAttr = c.emailAttribute || 'mail';
    const nameAttr = c.nameAttribute || 'displayName';
    const groupsAttr = c.groupsAttribute || 'memberOf';
    const idAttrs = c.idAttribute ? [c.idAttribute] : ['objectGUID', 'entryUUID'];
    const { searchEntries } = await client.search(c.baseDn, {
      scope: 'sub',
      filter: c.userFilter.replace(/\{username\}/g, escapeFilter(user)),
      attributes: [emailAttr, nameAttr, groupsAttr, ...idAttrs, 'cn'],
      sizeLimit: 2,
      explicitBufferAttributes: ['objectGUID'],
    });
    if (searchEntries.length === 0) throw trace.fail('search', 'Unknown user name or wrong password');
    if (searchEntries.length > 1) throw trace.fail('search', 'More than one directory entry matches this user name; ask an administrator to narrow the user filter');
    const entry = searchEntries[0];
    trace.ok('search', { dn: entry.dn });
    try {
      await client.bind(entry.dn, password);
    } catch {
      throw trace.fail('user_bind', 'Unknown user name or wrong password');
    }
    trace.ok('user_bind');
    const idRaw = idAttrs.map((a) => attr(entry, a)).find((v) => v !== undefined);
    const idValue = Array.isArray(idRaw) ? idRaw[0] : idRaw;
    const subject = idValue === undefined ? entry.dn.toLowerCase() : Buffer.isBuffer(idValue) ? idValue.toString('hex') : String(idValue);
    const groupDns = list(attr(entry, groupsAttr));
    const identity: IdpIdentity = {
      subject,
      email: text(attr(entry, emailAttr)).trim().toLowerCase(),
      emailVerified: true,
      name: text(attr(entry, nameAttr)) || text(attr(entry, 'cn')),
      // Mappings may use the full group DN or just its CN.
      groups: [...new Set([...groupDns, ...groupDns.map(groupName)])],
    };
    trace.ok('identity', { subject: identity.subject, email: identity.email, name: identity.name, groups: identity.groups.length });
    return identity;
  } catch (e) {
    if (e instanceof OrgError) throw e;
    throw trace.fail(trace.stage === 'start' ? 'connect' : trace.stage, `The directory could not be reached: ${(e as Error).message}`);
  } finally {
    await client.unbind().catch(() => {});
  }
}
