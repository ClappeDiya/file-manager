/**
 * Identity providers (OIDC / SAML) per organization, linked identities and
 * short-lived authorization state.
 */
import { db, newId, now, parseJson, type Row } from '../index';
import { openOpt, sealOpt } from '../../crypto';
import type { Role } from '../../types/auth';
import { ROLE_HIERARCHY } from '../../types/auth';
import { OrgError } from './orgs';
import { getGroup } from './groups';

export type SsoKind = 'oidc' | 'saml';

export interface OidcConfig {
  issuer: string;
  clientId: string;
  scopes: string;
  /** Accept tokens whose email is not marked verified (off by default). */
  allowUnverifiedEmail?: boolean;
  groupsClaim?: string;
}

export interface SamlConfig {
  /** IdP single sign-on URL (HTTP-Redirect binding). */
  entryPoint: string;
  /** IdP signing certificate(s), PEM. */
  idpCert: string;
  /** Expected IdP entity id (Issuer). */
  idpIssuer?: string;
  emailAttribute?: string;
  nameAttribute?: string;
  groupsAttribute?: string;
}

export interface SsoProvider {
  id: string;
  orgId: string;
  name: string;
  kind: SsoKind;
  config: OidcConfig | SamlConfig;
  hasSecret: boolean;
  defaultRole: Role;
  defaultGroupId: string | null;
  /** IdP group name → console group id, applied on first login. */
  groupMapping: Record<string, string>;
  requireMfa: boolean;
  allowJit: boolean;
  domains: string[];
  enabled: boolean;
  createdAt: string;
}

function fromRow(r: Row): SsoProvider {
  const cfg = parseJson<Record<string, unknown>>(r.config, {});
  const { groupMapping, ...config } = cfg;
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    kind: r.kind as SsoKind,
    config: config as unknown as OidcConfig | SamlConfig,
    hasSecret: !!r.secret_ciphertext,
    defaultRole: r.default_role as Role,
    defaultGroupId: (r.default_group_id as string) ?? null,
    groupMapping: (groupMapping as Record<string, string>) ?? {},
    requireMfa: Boolean(r.require_mfa),
    allowJit: Boolean(r.allow_jit),
    domains: parseJson<string[]>(r.domains, []),
    enabled: Boolean(r.enabled),
    createdAt: r.created_at as string,
  };
}

export function getProvider(id: string): SsoProvider | null {
  const r = db().prepare('SELECT * FROM sso_providers WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function providerSecret(id: string): string | null {
  return openOpt((db().prepare('SELECT secret_ciphertext s FROM sso_providers WHERE id = ?').get(id) as Row | undefined)?.s);
}

export function listProviders(orgId: string): SsoProvider[] {
  return (db().prepare('SELECT * FROM sso_providers WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(fromRow);
}

/** Enabled providers claiming an email domain (sign-in discovery). */
export function providersForDomain(domain: string): SsoProvider[] {
  const d = domain.trim().toLowerCase();
  return (db().prepare('SELECT * FROM sso_providers WHERE enabled = 1').all() as Row[]).map(fromRow).filter((p) => p.domains.includes(d));
}

export interface ProviderInput {
  name: string;
  kind: SsoKind;
  config: Record<string, unknown>;
  secret?: string | null;
  defaultRole?: Role;
  defaultGroupId?: string | null;
  groupMapping?: Record<string, string>;
  requireMfa?: boolean;
  allowJit?: boolean;
  domains?: string[];
  enabled?: boolean;
}

function validate(orgId: string, p: ProviderInput, existingSecret: boolean) {
  if (!p.name?.trim()) throw new OrgError('Give the provider a name');
  if (p.kind === 'oidc') {
    const c = p.config as unknown as OidcConfig;
    try {
      if (new URL(c.issuer).protocol !== 'https:' && !/^http:\/\/(localhost|127\.0\.0\.1)(:|\/)/.test(c.issuer)) throw new Error();
    } catch {
      throw new OrgError('The issuer must be an https URL');
    }
    if (!c.clientId?.trim()) throw new OrgError('Enter the client id');
    if (!p.secret && !existingSecret) throw new OrgError('Enter the client secret');
  } else if (p.kind === 'saml') {
    const c = p.config as unknown as SamlConfig;
    try {
      new URL(c.entryPoint);
    } catch {
      throw new OrgError('Enter the IdP sign-on URL');
    }
    if (!/-----BEGIN CERTIFICATE-----/.test(c.idpCert ?? '')) throw new OrgError('Paste the IdP signing certificate (PEM)');
  } else throw new OrgError('Provider type is oidc or saml');
  if (p.defaultRole && !(p.defaultRole in ROLE_HIERARCHY)) throw new OrgError('Unknown default role');
  if (p.defaultRole === 'org_owner') throw new OrgError('Automatic sign-up cannot grant the owner role');
  const groups = [p.defaultGroupId, ...Object.values(p.groupMapping ?? {})].filter(Boolean) as string[];
  for (const g of groups) if (getGroup(g)?.orgId !== orgId) throw new OrgError(`Group ${g} is not in this organization`);
  for (const d of p.domains ?? []) if (!/^[a-z0-9.-]+\.[a-z]{2,}$/i.test(d)) throw new OrgError(`"${d}" is not a domain name`);
}

function cleanDomains(d: string[] | undefined): string[] {
  return [...new Set((d ?? []).map((x) => x.trim().toLowerCase()).filter(Boolean))];
}

export function createProvider(orgId: string, p: ProviderInput): SsoProvider {
  validate(orgId, p, false);
  const id = newId('sso');
  db()
    .prepare('INSERT INTO sso_providers (id, org_id, name, kind, config, secret_ciphertext, default_role, default_group_id, require_mfa, allow_jit, domains, enabled, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, p.name.trim(), p.kind, JSON.stringify({ ...p.config, groupMapping: p.groupMapping ?? {} }), sealOpt(p.secret), p.defaultRole ?? 'user', p.defaultGroupId ?? null, p.requireMfa ? 1 : 0, p.allowJit === false ? 0 : 1, JSON.stringify(cleanDomains(p.domains)), p.enabled === false ? 0 : 1, now());
  return getProvider(id)!;
}

export function updateProvider(orgId: string, id: string, p: Partial<ProviderInput>): SsoProvider | null {
  const cur = getProvider(id);
  if (!cur || cur.orgId !== orgId) return null;
  const next: ProviderInput = {
    name: p.name ?? cur.name,
    kind: cur.kind,
    config: (p.config ?? cur.config) as Record<string, unknown>,
    secret: p.secret,
    defaultRole: p.defaultRole ?? cur.defaultRole,
    defaultGroupId: p.defaultGroupId === undefined ? cur.defaultGroupId : p.defaultGroupId,
    groupMapping: p.groupMapping ?? cur.groupMapping,
    requireMfa: p.requireMfa ?? cur.requireMfa,
    allowJit: p.allowJit ?? cur.allowJit,
    domains: p.domains ?? cur.domains,
    enabled: p.enabled ?? cur.enabled,
  };
  validate(orgId, next, cur.hasSecret);
  db()
    .prepare('UPDATE sso_providers SET name = ?, config = ?, default_role = ?, default_group_id = ?, require_mfa = ?, allow_jit = ?, domains = ?, enabled = ? WHERE id = ?')
    .run(next.name.trim(), JSON.stringify({ ...next.config, groupMapping: next.groupMapping }), next.defaultRole, next.defaultGroupId, next.requireMfa ? 1 : 0, next.allowJit ? 1 : 0, JSON.stringify(cleanDomains(next.domains)), next.enabled ? 1 : 0, id);
  if (p.secret !== undefined) db().prepare('UPDATE sso_providers SET secret_ciphertext = ? WHERE id = ?').run(sealOpt(p.secret), id);
  return getProvider(id);
}

export function deleteProvider(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM sso_providers WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

// ── Linked identities ──

export interface Identity {
  id: string;
  userId: string;
  providerId: string;
  subject: string;
  email: string;
  createdAt: string;
  lastUsedAt: string | null;
}

function idFromRow(r: Row): Identity {
  return { id: r.id as string, userId: r.user_id as string, providerId: r.provider_id as string, subject: r.subject as string, email: r.email as string, createdAt: r.created_at as string, lastUsedAt: (r.last_used_at as string) ?? null };
}

export function findIdentity(providerId: string, subject: string): Identity | null {
  const r = db().prepare('SELECT * FROM user_identities WHERE provider_id = ? AND subject = ?').get(providerId, subject) as Row | undefined;
  return r ? idFromRow(r) : null;
}

export function linkIdentity(userId: string, providerId: string, subject: string, email: string): Identity {
  const existing = findIdentity(providerId, subject);
  if (existing && existing.userId !== userId) throw new OrgError('This sign-in is already linked to another account', 409);
  if (!existing) db().prepare('INSERT INTO user_identities (id, user_id, provider_id, subject, email, created_at) VALUES (?, ?, ?, ?, ?, ?)').run(newId('idn'), userId, providerId, subject, email, now());
  db().prepare('UPDATE user_identities SET last_used_at = ?, email = ? WHERE provider_id = ? AND subject = ?').run(now(), email, providerId, subject);
  return findIdentity(providerId, subject)!;
}

export function identitiesOf(userId: string): (Identity & { providerName: string; kind: SsoKind })[] {
  return (db().prepare('SELECT i.*, p.name AS provider_name, p.kind AS kind FROM user_identities i JOIN sso_providers p ON p.id = i.provider_id WHERE i.user_id = ? ORDER BY i.created_at').all(userId) as Row[]).map((r) => ({ ...idFromRow(r), providerName: r.provider_name as string, kind: r.kind as SsoKind }));
}

export function unlinkIdentity(userId: string, id: string): boolean {
  return db().prepare('DELETE FROM user_identities WHERE user_id = ? AND id = ?').run(userId, id).changes > 0;
}

// ── Authorization state (OIDC state / nonce / PKCE; 10 minutes) ──

export interface AuthState {
  state: string;
  providerId: string;
  nonce: string;
  codeVerifier: string;
  redirect: string;
  linkUserId: string | null;
}

export function saveState(s: AuthState): void {
  db().prepare("DELETE FROM auth_states WHERE created_at < ?").run(new Date(Date.now() - 600_000).toISOString());
  db().prepare('INSERT INTO auth_states (state, provider_id, nonce, code_verifier, redirect, link_user_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)').run(s.state, s.providerId, s.nonce, s.codeVerifier, s.redirect, s.linkUserId, now());
}

/** Read and delete (single use). */
export function takeState(state: string): AuthState | null {
  const r = db().prepare('SELECT * FROM auth_states WHERE state = ? AND created_at >= ?').get(state, new Date(Date.now() - 600_000).toISOString()) as Row | undefined;
  db().prepare('DELETE FROM auth_states WHERE state = ?').run(state);
  return r ? { state: r.state as string, providerId: r.provider_id as string, nonce: r.nonce as string, codeVerifier: r.code_verifier as string, redirect: r.redirect as string, linkUserId: (r.link_user_id as string) ?? null } : null;
}

/** node-saml cache for AuthnRequest ids (InResponseTo validation across processes). */
export const samlRequestCache = {
  async saveAsync(key: string, value: string) {
    const createdAt = Date.now();
    db().prepare('INSERT OR REPLACE INTO saml_requests (id, value, created_at) VALUES (?, ?, ?)').run(key, value, new Date(createdAt).toISOString());
    return { value, createdAt };
  },
  async getAsync(key: string) {
    const r = db().prepare('SELECT value FROM saml_requests WHERE id = ? AND created_at >= ?').get(key, new Date(Date.now() - 600_000).toISOString()) as Row | undefined;
    return (r?.value as string) ?? null;
  },
  async removeAsync(key: string | null) {
    if (!key) return null;
    const r = db().prepare('SELECT value FROM saml_requests WHERE id = ?').get(key) as Row | undefined;
    db().prepare('DELETE FROM saml_requests WHERE id = ?').run(key);
    return (r?.value as string) ?? null;
  },
};

// ── Sign-in diagnostics ──

/** One step of an SSO sign-in, with what the provider sent (no tokens or signatures). */
export interface SsoStep {
  step: string;
  ok: boolean;
  detail?: Record<string, unknown>;
}

export interface SsoAttempt {
  id: string;
  providerId: string;
  orgId: string;
  kind: SsoKind;
  at: string;
  ok: boolean;
  /** The step that failed, or `done`. */
  stage: string;
  error: string | null;
  steps: SsoStep[];
  email: string;
  userId: string | null;
  ip: string;
}

/** Attempts kept per provider. */
export const KEEP_ATTEMPTS = 200;

export function recordSsoAttempt(p: Pick<SsoProvider, 'id' | 'orgId' | 'kind'>, a: { ok: boolean; stage: string; error: string | null; steps: SsoStep[]; email: string; userId: string | null; ip: string }): void {
  db()
    .prepare('INSERT INTO sso_attempts (id, provider_id, org_id, kind, at, ok, stage, error, steps, email, user_id, ip) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(newId('ssa'), p.id, p.orgId, p.kind, now(), a.ok ? 1 : 0, a.stage, a.error, JSON.stringify(a.steps), a.email, a.userId, a.ip);
  db()
    .prepare('DELETE FROM sso_attempts WHERE provider_id = ? AND id NOT IN (SELECT id FROM sso_attempts WHERE provider_id = ? ORDER BY at DESC, rowid DESC LIMIT ?)')
    .run(p.id, p.id, KEEP_ATTEMPTS);
}

export function listSsoAttempts(orgId: string, providerId: string, limit = 50): SsoAttempt[] {
  return (db().prepare('SELECT * FROM sso_attempts WHERE org_id = ? AND provider_id = ? ORDER BY at DESC, rowid DESC LIMIT ?').all(orgId, providerId, Math.min(limit, KEEP_ATTEMPTS)) as Row[]).map((r) => ({
    id: r.id as string,
    providerId: r.provider_id as string,
    orgId: r.org_id as string,
    kind: r.kind as SsoKind,
    at: r.at as string,
    ok: !!r.ok,
    stage: r.stage as string,
    error: (r.error as string) ?? null,
    steps: parseJson<SsoStep[]>(r.steps, []),
    email: r.email as string,
    userId: (r.user_id as string) ?? null,
    ip: r.ip as string,
  }));
}
