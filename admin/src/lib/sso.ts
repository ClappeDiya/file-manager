/**
 * Single sign-on flows.
 *
 * OIDC: authorization code with PKCE (S256), `state` and `nonce`; the ID
 * token is verified against the issuer's JWKS (signature, issuer, audience,
 * expiry, nonce). SAML: SP-initiated, HTTP-Redirect AuthnRequest, signed
 * assertions required, InResponseTo checked against issued request ids.
 *
 * After the IdP proves who the user is, `resolveUser` maps the identity to a
 * console account: an existing linked identity, an explicit link from a
 * signed-in user, an existing account with the same verified email, or —
 * when allowed — a new account created on first login with the provider's
 * default role and groups (plus groups mapped from the IdP's group claim).
 */
import crypto from 'crypto';
import { createRemoteJWKSet, jwtVerify, type JWTPayload } from 'jose';
import { SAML, ValidateInResponseTo } from '@node-saml/node-saml';
import { getProvider, providerSecret, samlRequestCache, saveState, findIdentity, linkIdentity, type OidcConfig, type SamlConfig, type SsoProvider } from './db/repos/sso';
import { createUser, findUserByEmail, getUser, type UserRecord } from './db/repos/users';
import { effectiveRole, OrgError, setMembership } from './db/repos/orgs';
import { getGroup, setGroupMember } from './db/repos/groups';
import { appendAudit } from './db/repos/audit';

interface Discovery {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  jwks_uri: string;
  userinfo_endpoint?: string;
}

const discoveryCache = new Map<string, { at: number; doc: Discovery }>();
const jwksCache = new Map<string, ReturnType<typeof createRemoteJWKSet>>();

export async function discover(issuer: string): Promise<Discovery> {
  const key = issuer.replace(/\/$/, '');
  const hit = discoveryCache.get(key);
  if (hit && Date.now() - hit.at < 3_600_000) return hit.doc;
  const res = await fetch(`${key}/.well-known/openid-configuration`, { signal: AbortSignal.timeout(10_000) });
  if (!res.ok) throw new OrgError(`The identity provider's discovery document returned HTTP ${res.status}`, 502);
  const doc = (await res.json()) as Discovery;
  if (doc.issuer?.replace(/\/$/, '') !== key) throw new OrgError('The discovery document names a different issuer', 502);
  discoveryCache.set(key, { at: Date.now(), doc });
  return doc;
}

const b64url = (b: Buffer) => b.toString('base64url');

export const callbackUrl = (origin: string, providerId: string) => `${origin}/api/auth/sso/${providerId}/callback`;

function samlClient(p: SsoProvider, origin: string): SAML {
  const c = p.config as SamlConfig;
  return new SAML({
    entryPoint: c.entryPoint,
    idpCert: c.idpCert,
    idpIssuer: c.idpIssuer || undefined,
    issuer: `${origin}/api/auth/sso/${p.id}/metadata`,
    callbackUrl: callbackUrl(origin, p.id),
    audience: `${origin}/api/auth/sso/${p.id}/metadata`,
    wantAssertionsSigned: true,
    wantAuthnResponseSigned: false,
    validateInResponseTo: ValidateInResponseTo.always,
    cacheProvider: samlRequestCache,
    requestIdExpirationPeriodMs: 600_000,
    acceptedClockSkewMs: 120_000,
    identifierFormat: 'urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress',
  });
}

export function samlMetadata(p: SsoProvider, origin: string): string {
  return samlClient(p, origin).generateServiceProviderMetadata(null);
}

/** Where to send the browser to start signing in. */
export async function startUrl(p: SsoProvider, origin: string, redirect: string, linkUserId: string | null): Promise<string> {
  if (!p.enabled) throw new OrgError('This sign-in method is disabled', 403);
  const state = b64url(crypto.randomBytes(24));
  if (p.kind === 'oidc') {
    const c = p.config as OidcConfig;
    const d = await discover(c.issuer);
    const nonce = b64url(crypto.randomBytes(16));
    const verifier = b64url(crypto.randomBytes(32));
    saveState({ state, providerId: p.id, nonce, codeVerifier: verifier, redirect, linkUserId });
    const u = new URL(d.authorization_endpoint);
    u.search = new URLSearchParams({
      response_type: 'code',
      client_id: c.clientId,
      redirect_uri: callbackUrl(origin, p.id),
      scope: c.scopes?.trim() || 'openid email profile',
      state,
      nonce,
      code_challenge: b64url(crypto.createHash('sha256').update(verifier).digest()),
      code_challenge_method: 'S256',
    }).toString();
    return u.toString();
  }
  // SAML carries our state in RelayState; the request id is cached by node-saml.
  saveState({ state, providerId: p.id, nonce: '', codeVerifier: '', redirect, linkUserId });
  return samlClient(p, origin).getAuthorizeUrlAsync(state, undefined, {});
}

export interface IdpIdentity {
  subject: string;
  email: string;
  emailVerified: boolean;
  name: string;
  groups: string[];
}

/** OIDC callback: exchange the code and verify the ID token. */
export async function oidcIdentity(p: SsoProvider, origin: string, code: string, st: { nonce: string; codeVerifier: string }): Promise<IdpIdentity> {
  const c = p.config as OidcConfig;
  const d = await discover(c.issuer);
  const secret = providerSecret(p.id) ?? '';
  const res = await fetch(d.token_endpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded', authorization: `Basic ${Buffer.from(`${encodeURIComponent(c.clientId)}:${encodeURIComponent(secret)}`).toString('base64')}` },
    body: new URLSearchParams({ grant_type: 'authorization_code', code, redirect_uri: callbackUrl(origin, p.id), code_verifier: st.codeVerifier }).toString(),
    signal: AbortSignal.timeout(15_000),
  });
  const tok = (await res.json().catch(() => ({}))) as { id_token?: string; access_token?: string; error?: string; error_description?: string };
  if (!res.ok || !tok.id_token) throw new OrgError(`Sign-in failed at the identity provider: ${tok.error_description ?? tok.error ?? `HTTP ${res.status}`}`, 401);
  let jwks = jwksCache.get(d.jwks_uri);
  if (!jwks) jwksCache.set(d.jwks_uri, (jwks = createRemoteJWKSet(new URL(d.jwks_uri))));
  let claims: JWTPayload & Record<string, unknown>;
  try {
    claims = (await jwtVerify(tok.id_token, jwks, { issuer: d.issuer, audience: c.clientId, clockTolerance: 60 })).payload as JWTPayload & Record<string, unknown>;
  } catch (e) {
    throw new OrgError(`The identity token could not be verified: ${(e as Error).message}`, 401);
  }
  if (claims.nonce !== st.nonce) throw new OrgError('The identity token does not belong to this sign-in (nonce mismatch)', 401);
  const groupsClaim = c.groupsClaim || 'groups';
  // Conforming providers keep scope claims out of the ID token when an access
  // token is issued; fetch them from userinfo (same subject required).
  if ((claims.email === undefined || claims[groupsClaim] === undefined) && d.userinfo_endpoint && tok.access_token) {
    const ui = await fetch(d.userinfo_endpoint, { headers: { authorization: `Bearer ${tok.access_token}` }, signal: AbortSignal.timeout(10_000) });
    const info = ui.ok ? ((await ui.json().catch(() => ({}))) as Record<string, unknown>) : {};
    if (info.sub === claims.sub) for (const [k, v] of Object.entries(info)) if (claims[k] === undefined) claims[k] = v;
  }
  const g = claims[groupsClaim];
  return {
    subject: String(claims.sub ?? ''),
    email: String(claims.email ?? '').toLowerCase(),
    emailVerified: claims.email_verified === true || claims.email_verified === 'true',
    name: String(claims.name ?? claims.preferred_username ?? ''),
    groups: Array.isArray(g) ? g.map(String) : typeof g === 'string' ? [g] : [],
  };
}

/** SAML ACS: validate the posted response. Returns the identity and our RelayState. */
export async function samlIdentity(p: SsoProvider, origin: string, body: Record<string, string>): Promise<{ identity: IdpIdentity; relayState: string }> {
  const c = p.config as SamlConfig;
  let profile;
  try {
    ({ profile } = await samlClient(p, origin).validatePostResponseAsync(body));
  } catch (e) {
    throw new OrgError(`The SAML response was rejected: ${(e as Error).message}`, 401);
  }
  if (!profile) throw new OrgError('The SAML response contained no assertion', 401);
  const attr = (name: string | undefined) => (name ? profile[name] : undefined);
  const email = String(attr(c.emailAttribute) ?? profile.email ?? profile.mail ?? profile['urn:oid:0.9.2342.19200300.100.1.3'] ?? (profile.nameID?.includes('@') ? profile.nameID : '')).toLowerCase();
  const groups = attr(c.groupsAttribute || 'groups');
  return {
    identity: { subject: profile.nameID, email, emailVerified: !!email, name: String(attr(c.nameAttribute || 'displayName') ?? ''), groups: Array.isArray(groups) ? groups.map(String) : typeof groups === 'string' ? [groups] : [] },
    relayState: body.RelayState ?? '',
  };
}

/** Map the IdP identity to a console account (see the module comment). */
export function resolveUser(p: SsoProvider, id: IdpIdentity, linkUserId: string | null, ip: string): UserRecord {
  if (!id.subject) throw new OrgError('The identity provider sent no subject', 401);
  const audit = (description: string, userId: string, email: string, eventType = 'auth.sso') => appendAudit({ eventType, orgId: p.orgId, userId, userEmail: email, ipAddress: ip, description, metadata: { providerId: p.id } });
  // Explicit link from an already signed-in user (linked login methods).
  if (linkUserId) {
    const u = getUser(linkUserId);
    if (!u?.isActive) throw new OrgError('Sign in again before linking', 401);
    linkIdentity(u.id, p.id, id.subject, id.email);
    audit(`Linked ${p.name} sign-in to the account`, u.id, u.email, 'auth.identity_link');
    return u;
  }
  const known = findIdentity(p.id, id.subject);
  if (known) {
    const u = getUser(known.userId);
    if (!u?.isActive) throw new OrgError('Account is deactivated', 403);
    linkIdentity(u.id, p.id, id.subject, id.email);
    return u;
  }
  const c = p.config as OidcConfig;
  if (!id.email) throw new OrgError('The identity provider did not send an email address', 401);
  if (!id.emailVerified && !(p.kind === 'oidc' && c.allowUnverifiedEmail)) throw new OrgError('The identity provider did not verify this email address', 401);
  const domain = id.email.split('@')[1] ?? '';
  const domainOk = p.domains.length === 0 || p.domains.includes(domain);
  const existing = findUserByEmail(id.email);
  if (existing) {
    // Automatic linking only for addresses in a domain the provider owns.
    if (!domainOk || p.domains.length === 0) throw new OrgError('An account with this email exists. Sign in with your password and link this method from your profile.', 409);
    if (!existing.isActive) throw new OrgError('Account is deactivated', 403);
    linkIdentity(existing.id, p.id, id.subject, id.email);
    if (!effectiveRole(existing.id, p.orgId)) setMembership(existing.id, p.orgId, p.defaultRole);
    audit(`Linked ${p.name} sign-in by verified email`, existing.id, existing.email, 'auth.identity_link');
    return existing;
  }
  if (!p.allowJit) throw new OrgError('Ask an administrator to invite you first', 403);
  if (!domainOk) throw new OrgError('This email domain cannot sign up through this provider', 403);
  const u = createUser({ email: id.email, name: id.name || id.email.split('@')[0], role: p.defaultRole, organizationId: p.orgId, passwordHash: null, ssoProvider: p.kind === 'saml' ? 'saml' : null });
  setMembership(u.id, p.orgId, p.defaultRole);
  // First-login group assignment: the provider's default group plus mapped IdP groups.
  const groups = new Set<string>();
  if (p.defaultGroupId) groups.add(p.defaultGroupId);
  for (const g of id.groups) if (p.groupMapping[g]) groups.add(p.groupMapping[g]);
  for (const gid of groups) if (getGroup(gid)?.orgId === p.orgId) setGroupMember(gid, u.id, 'member');
  linkIdentity(u.id, p.id, id.subject, id.email);
  audit(`Created account on first ${p.name} sign-in (${p.defaultRole}, ${groups.size} group(s))`, u.id, u.email, 'auth.sso_signup');
  return u;
}

export function providerOrThrow(id: string): SsoProvider {
  const p = getProvider(id);
  if (!p) throw new OrgError('Unknown sign-in method', 404);
  return p;
}
