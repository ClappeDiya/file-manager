/** LDAP provider settings and their checks (no I/O; shared by the provider repository and `ldap.ts`). */

export interface LdapConfig {
  /** `ldaps://dc.example.com:636` or `ldap://…` with `startTls`. */
  url: string;
  startTls?: boolean;
  /** PEM certificate(s) of the directory's CA, when not publicly trusted. */
  caCert?: string;
  /** Service account that may search for users. */
  bindDn: string;
  baseDn: string;
  /** Search filter with `{username}`, e.g. `(&(objectClass=user)(sAMAccountName={username}))`. */
  userFilter: string;
  emailAttribute?: string;
  nameAttribute?: string;
  groupsAttribute?: string;
  /** Stable id attribute (`objectGUID` on AD, `entryUUID` on OpenLDAP); default: tries both, then the DN. */
  idAttribute?: string;
}

const LOOPBACK = /^ldap:\/\/(localhost|127\.\d+\.\d+\.\d+|\[::1\])(:\d+)?\/?$/i;

/** Problems with a configuration (shown when saving the provider). */
export function validateLdapConfig(c: Partial<LdapConfig>): string | null {
  let u: URL;
  try {
    u = new URL(c.url ?? '');
  } catch {
    return 'Enter the directory URL (ldaps://host:636 or ldap://host with StartTLS)';
  }
  if (u.protocol !== 'ldaps:' && u.protocol !== 'ldap:') return 'The directory URL starts with ldaps:// or ldap://';
  if (u.protocol === 'ldap:' && !c.startTls && !LOOPBACK.test(c.url ?? '')) return 'Plain ldap:// sends passwords unencrypted: use ldaps:// or turn on StartTLS';
  if (!c.bindDn?.trim()) return 'Enter the service account DN used to find users';
  if (!c.baseDn?.trim()) return 'Enter the base DN to search for users';
  if (!c.userFilter?.includes('{username}')) return 'The user filter must contain {username}';
  if (c.caCert && !/-----BEGIN CERTIFICATE-----/.test(c.caCert)) return 'The CA certificate must be PEM';
  return null;
}
