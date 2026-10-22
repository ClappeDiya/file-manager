/**
 * Sessions and authorization.
 *
 * The session cookie is an HS256 JWT with organization-scoped claims:
 * `sub` (email), `uid`, `sid` (the server-side session), `org` (current
 * organization), `amr` (how the user signed in: pwd / sso / passkey /
 * mfa). The role is *not* trusted from the token: it is recomputed from
 * memberships on every request, so role changes, deactivation and removed
 * memberships apply immediately. The session row must still be valid
 * (not revoked, not idle past the organization's limit).
 */
import { NextRequest, NextResponse } from 'next/server';
import { SignJWT, jwtVerify } from 'jose';
import { ROLE_PERMISSIONS, type Permission, type Role } from './types/auth';
import { getUser, type UserRecord } from './db/repos/users';
import { accessibleOrgs, effectiveRole, getOrg } from './db/repos/orgs';
import { getSecurity, needsSecondFactorSetup, sessionIdleMinutes } from './db/repos/security';
import { clientIp, ipAllowed } from './netacl';
import { createSession, getSession, revokeSession, stopViewAs, touchSession } from './db/repos/sessions';
import { hasPasskeys } from './db/repos/passkeys';
import { isBearerToken, peekApiToken, useApiToken } from './db/repos/api-tokens';
import { logApiRequest } from './db/repos/api-log';
import { appendAudit, type AuditInput } from './db/repos/audit';

export const SESSION_COOKIE = 'ufop-admin-session';
const SESSION_HOURS = 8;

export interface AuthContext {
  user: UserRecord;
  orgId: string;
  role: Role;
  permissions: Permission[];
  amr: string[];
  /** The server-side session id. */
  sid: string;
  /** When the user last signed in or re-authenticated in this session. */
  verifiedAt: string;
  /** Set when the request used an API token instead of a session. */
  tokenId: string | null;
  ip: string;
  userAgent: string;
  /** Set while an administrator views the console as `user` (read-only). */
  viewAs?: { actor: UserRecord; until: string } | null;
}

/** What may still be changed while viewing as someone: stopping, and signing out. */
const VIEW_AS_WRITABLE = ['/api/view-as', '/api/auth/logout'];



function secret(): Uint8Array {
  const s = process.env.NEXTAUTH_SECRET;
  if (!s) throw new Error('NEXTAUTH_SECRET is not set');
  return new TextEncoder().encode(s);
}

export { clientIp };

/** Sign a session cookie; without `sid` a new server-side session is created. */
export async function signSession(user: UserRecord, orgId: string, amr: string[], sid?: string, meta: { ip?: string; userAgent?: string } = {}): Promise<string> {
  const id = sid ?? createSession(user.id, { method: amr.join('+'), ip: meta.ip, userAgent: meta.userAgent, hours: SESSION_HOURS }).id;
  return new SignJWT({ name: user.name, uid: user.id, sid: id, org: orgId, amr })
    .setProtectedHeader({ alg: 'HS256' })
    .setSubject(user.email)
    .setIssuedAt()
    .setExpirationTime(`${SESSION_HOURS}h`)
    .sign(secret());
}

/** Put a session cookie on a response. */
export async function attachSession<T extends NextResponse>(res: T, user: UserRecord, orgId: string, amr: string[], sid?: string, meta: { ip?: string; userAgent?: string } = {}): Promise<T> {
  res.cookies.set(SESSION_COOKIE, await signSession(user, orgId, amr, sid, meta), {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'lax',
    maxAge: SESSION_HOURS * 3600,
    path: '/',
  });
  return res;
}

/** The organization a user lands in: their home org if reachable, else the first accessible one. */
export function defaultOrgFor(user: UserRecord): string | null {
  if (effectiveRole(user.id, user.organizationId)) return user.organizationId;
  const all = accessibleOrgs(user.id);
  return all[0]?.org.id ?? null;
}

const deny = (status: number, error: string) => NextResponse.json({ error }, { status });

/** What a session that still has to set up required MFA may call. */
const MFA_SETUP_PATHS = ['/api/account/mfa', '/api/account/passkeys', '/api/auth/session', '/api/auth/logout'];

/** Paths an API token may not use: account and sign-in changes, minting tokens, switching organization. */
const TOKEN_DENIED_PATHS = ['/api/account/', '/api/auth/', '/api/api-tokens', '/api/orgs/switch', '/api/invitations/'];

export async function authenticate(req: NextRequest): Promise<AuthContext | NextResponse> {
  const token = req.cookies.get(SESSION_COOKIE)?.value;
  // A bearer token is only used without a session cookie (scripts, not browsers).
  if (!token && isBearerToken(req.headers.get('authorization'))) return authenticateToken(req);
  if (!process.env.NEXTAUTH_SECRET) return deny(500, 'Server configuration error');
  if (!token) return deny(401, 'Unauthorized');
  let payload: Record<string, unknown>;
  try {
    payload = (await jwtVerify(token, secret())).payload as Record<string, unknown>;
  } catch {
    return deny(401, 'Session expired');
  }
  const user = typeof payload.uid === 'string' ? getUser(payload.uid) : null;
  if (!user || !user.isActive) return deny(401, 'Account is no longer active');
  const session = typeof payload.sid === 'string' ? getSession(payload.sid) : null;
  if (!session || session.userId !== user.id || session.revokedAt) return deny(401, 'This session has ended; sign in again');
  const idle = sessionIdleMinutes(user.id);
  if (idle && Date.now() - new Date(session.lastSeenAt).getTime() > idle * 60_000) {
    revokeSession(session.id, 'idle');
    return deny(401, 'Signed out after inactivity; sign in again');
  }
  touchSession(session);
  const amr = Array.isArray(payload.amr) ? (payload.amr as string[]) : ['pwd'];
  const orgId = typeof payload.org === 'string' ? payload.org : user.organizationId;
  const admitted = admit(req, user, orgId, amr.includes('sso'));
  if ('denied' in admitted) {
    // Refusals by the organization's network rule are logged (two-factor setup is a normal flow).
    if (admitted.reason === 'network') logRequest(req, 403, 'network not allowed', { orgId, tokenId: null, user, auth: 'session' });
    return admitted.denied;
  }
  const ctx: AuthContext = { user, orgId, role: admitted.role, permissions: ROLE_PERMISSIONS[admitted.role] ?? [], amr, sid: session.id, verifiedAt: session.verifiedAt, tokenId: null, ip: admitted.ip, userAgent: req.headers.get('user-agent') ?? '' };
  return session.viewAsUserId ? viewingAs(req, ctx, session) : ctx;
}

/**
 * The context while an administrator views the console as another member:
 * that member's identity and role, read permissions only, every change
 * refused, and every request logged with the administrator's name. Ends by
 * itself at `viewAsUntil`.
 */
function viewingAs(req: NextRequest, ctx: AuthContext, session: { id: string; viewAsUserId: string | null; viewAsUntil: string | null }): AuthContext | NextResponse {
  const target = session.viewAsUserId ? getUser(session.viewAsUserId) : null;
  const role = target?.isActive ? effectiveRole(target.id, ctx.orgId) : null;
  if (!target || !role || !session.viewAsUntil || Date.parse(session.viewAsUntil) <= Date.now()) {
    stopViewAs(session.id);
    appendAudit({ orgId: ctx.orgId, userId: ctx.user.id, userEmail: ctx.user.email, userName: ctx.user.name, ipAddress: ctx.ip, userAgent: ctx.userAgent, eventType: 'user.view_as_end', description: `Stopped viewing as ${target?.email ?? 'a member'} (${!target || !role ? 'no longer a member' : 'time limit'})`, resourceType: 'user', resourceId: session.viewAsUserId ?? '' });
    return ctx;
  }
  const path = req.nextUrl.pathname;
  const writable = VIEW_AS_WRITABLE.some((p) => path === p || path.startsWith(`${p}/`));
  logRequest(req, 200, `viewing as ${target.email}`, { orgId: ctx.orgId, tokenId: null, user: ctx.user, auth: 'session' });
  if (!['GET', 'HEAD'].includes(req.method) && !writable) {
    return NextResponse.json({ error: `Read-only: you are viewing the console as ${target.email}. Stop viewing as them to make changes.`, code: 'view_as_read_only' }, { status: 403 });
  }
  const permissions = (ROLE_PERMISSIONS[role] ?? []).filter((p) => p === 'admin.access' || p.endsWith('.read'));
  return { ...ctx, user: target, role, permissions, viewAs: { actor: ctx.user, until: session.viewAsUntil } };
}

/**
 * The checks every request passes, whichever way it authenticated: access
 * to the organization, its allowed networks and its two-factor rule.
 */
type Admission = { role: Role; ip: string } | { denied: NextResponse; reason: 'access' | 'network' | 'two_factor' };

function admit(req: NextRequest, user: UserRecord, orgId: string, viaSso: boolean): Admission {
  const org = getOrg(orgId);
  const role = org ? effectiveRole(user.id, orgId) : null;
  if (!org || !role) return { denied: deny(403, 'You no longer have access to this organization'), reason: 'access' };
  const ip = clientIp(req);
  // The organization only admits console access from its networks (signing out still works).
  if (!ipAllowed(getSecurity(orgId).ipAllowlist, ip) && !req.nextUrl.pathname.startsWith('/api/auth/logout')) {
    return { denied: NextResponse.json({ error: `This organization does not allow console access from ${ip}`, code: 'ip_not_allowed' }, { status: 403 }), reason: 'network' };
  }
  // An organization requires two-factor sign-in and this account has none yet:
  // the session can only set it up (also for sessions from before the rule).
  if (needsSecondFactorSetup(user, viaSso) && !MFA_SETUP_PATHS.some((p) => req.nextUrl.pathname.startsWith(p))) {
    return { denied: NextResponse.json({ error: 'Your organization requires two-factor sign-in. Set it up under My account first.', code: 'mfa_setup_required' }, { status: 403 }), reason: 'two_factor' };
  }
  return { role, ip };
}

/** Record an API-token request, or a refused one (see `db/repos/api-log`). */
function logRequest(req: NextRequest, status: number, outcome: string, who: { orgId: string; tokenId: string | null; user: UserRecord | null; auth: 'token' | 'session' }): void {
  logApiRequest({ orgId: who.orgId, method: req.method, path: req.nextUrl.pathname, status, outcome, auth: who.auth, tokenId: who.tokenId, userId: who.user?.id ?? null, userEmail: who.user?.email ?? '', ip: clientIp(req), userAgent: req.headers.get('user-agent') ?? '' });
}

/** `Authorization: Bearer ufop_cat_…`: the token's permissions, narrowed to what its owner's role allows now. */
function authenticateToken(req: NextRequest): AuthContext | NextResponse {
  const secret = (req.headers.get('authorization') ?? '').slice('Bearer '.length).trim();
  const t = useApiToken(secret, clientIp(req));
  const refuse = (res: NextResponse, outcome: string, tok = t ?? peekApiToken(secret)) => {
    logRequest(req, res.status, outcome, { orgId: tok?.orgId ?? '', tokenId: tok?.id ?? null, user: tok ? getUser(tok.userId) : null, auth: 'token' });
    return res;
  };
  if (!t) return refuse(deny(401, 'Unknown, expired or revoked API token'), 'unknown, expired or revoked token');
  if (TOKEN_DENIED_PATHS.some((p) => req.nextUrl.pathname.startsWith(p))) return refuse(deny(403, 'API tokens cannot be used for this; sign in to the console'), 'not allowed with a token');
  const user = getUser(t.userId);
  if (!user || !user.isActive) return refuse(deny(401, 'The token owner’s account is no longer active'), 'owner inactive');
  const admitted = admit(req, user, t.orgId, false);
  if ('denied' in admitted) return refuse(admitted.denied, { access: 'no access to the organization', network: 'network not allowed', two_factor: 'owner must set up two-factor sign-in' }[admitted.reason]);
  const now = ROLE_PERMISSIONS[admitted.role] ?? [];
  return {
    user,
    orgId: t.orgId,
    role: admitted.role,
    permissions: t.permissions.filter((p) => now.includes(p)),
    amr: ['token'],
    sid: `token:${t.id}`,
    verifiedAt: new Date(0).toISOString(),
    tokenId: t.id,
    ip: admitted.ip,
    userAgent: req.headers.get('user-agent') ?? '',
  };
}

export function isContext(x: AuthContext | NextResponse): x is AuthContext {
  return !(x instanceof NextResponse);
}

export function can(ctx: AuthContext, p: Permission): boolean {
  return ctx.permissions.includes(p);
}

/** Authenticate and require every listed permission in the current organization. */
export async function requirePermission(req: NextRequest, ...perms: Permission[]): Promise<AuthContext | NextResponse> {
  const ctx = await authenticate(req);
  if (!isContext(ctx)) return ctx;
  const missing = can(ctx, 'admin.access') ? perms.filter((p) => !can(ctx, p)) : ['admin.access' as Permission];
  // API-token traffic is logged with its outcome.
  if (ctx.tokenId) logRequest(req, missing.length ? 403 : 200, missing.length ? `missing ${missing.join(', ')}` : 'authorized', { orgId: ctx.orgId, tokenId: ctx.tokenId, user: ctx.user, auth: 'token' });
  if (missing.includes('admin.access')) return deny(403, 'Your role cannot use the admin console');
  if (missing.length) return deny(403, `Forbidden: requires ${missing.join(', ')}`);
  return ctx;
}

/** Sensitive changes need a sign-in or re-authentication this recent (STEP_UP_MINUTES, default 10). */
export function stepUpMinutes(): number {
  const n = Number(process.env.STEP_UP_MINUTES ?? 10);
  return Number.isFinite(n) && n > 0 ? n : 10;
}

/** How this account can prove itself again: its strongest factors, else its password, else single sign-on. */
export function reauthMethods(user: UserRecord): ReauthMethod[] {
  const strong: ReauthMethod[] = [...(user.mfaEnabled ? (['totp'] as const) : []), ...(hasPasskeys(user.id) ? (['passkey'] as const) : [])];
  if (strong.length) return strong;
  return user.passwordHash ? ['password'] : ['sso'];
}
export type ReauthMethod = 'totp' | 'passkey' | 'password' | 'sso';

/**
 * Like `requirePermission`, for sensitive changes: the session must have
 * signed in or re-authenticated recently, else 403 `reauth_required` (the
 * console asks the user to confirm and retries).
 */
export async function requireStepUp(req: NextRequest, ...perms: Permission[]): Promise<AuthContext | NextResponse> {
  const ctx = await requirePermission(req, ...perms);
  if (!isContext(ctx)) return ctx;
  return recentlyVerified(ctx) ? ctx : stepUpResponse(ctx);
}

export function recentlyVerified(ctx: AuthContext, at = Date.now()): boolean {
  return at - new Date(ctx.verifiedAt).getTime() <= stepUpMinutes() * 60_000;
}

export function stepUpResponse(ctx: AuthContext): NextResponse {
  if (ctx.tokenId) return NextResponse.json({ error: 'API tokens cannot make this change; use the console', code: 'interactive_required' }, { status: 403 });
  return NextResponse.json({ error: 'Confirm it is you to make this change', code: 'reauth_required', methods: reauthMethods(ctx.user) }, { status: 403 });
}

/** Permission check against another organization the user can reach. */
export function canIn(ctx: AuthContext, orgId: string, p: Permission): boolean {
  if (orgId === ctx.orgId) return can(ctx, p);
  const role = effectiveRole(ctx.user.id, orgId);
  return !!role && (ROLE_PERMISSIONS[role] ?? []).includes(p);
}

export function audit(ctx: AuthContext | null, e: AuditInput): void {
  // While viewing as someone, the administrator is the one acting.
  const who = ctx?.viewAs?.actor ?? ctx?.user;
  appendAudit({
    orgId: ctx?.orgId,
    userId: who?.id,
    userEmail: who?.email,
    userName: who?.name,
    ipAddress: ctx?.ip,
    userAgent: ctx?.userAgent,
    ...e,
    // Changes made with an API token name it.
    ...(ctx?.tokenId ? { metadata: { ...e.metadata, apiToken: ctx.tokenId } } : {}),
  });
}

export async function readJson<T = Record<string, unknown>>(req: Request): Promise<T | null> {
  try {
    return (await req.json()) as T;
  } catch {
    return null;
  }
}

export function badRequest(error: string, status = 400) {
  return NextResponse.json({ error }, { status });
}
