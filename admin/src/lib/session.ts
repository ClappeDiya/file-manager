/**
 * Sessions and authorization.
 *
 * The session cookie is an HS256 JWT with organization-scoped claims:
 * `sub` (email), `uid`, `org` (current organization), `amr` (how the user
 * signed in: pwd / sso / mfa). The role is *not* trusted from the token: it
 * is recomputed from memberships on every request, so role changes,
 * deactivation and removed memberships apply immediately.
 */
import { NextRequest, NextResponse } from 'next/server';
import { SignJWT, jwtVerify } from 'jose';
import { ROLE_PERMISSIONS, type Permission, type Role } from './types/auth';
import { getUser, type UserRecord } from './db/repos/users';
import { accessibleOrgs, effectiveRole, getOrg } from './db/repos/orgs';
import { mfaRequired } from './db/repos/security';
import { appendAudit, type AuditInput } from './db/repos/audit';

export const SESSION_COOKIE = 'ufop-admin-session';
const SESSION_HOURS = 8;

export interface AuthContext {
  user: UserRecord;
  orgId: string;
  role: Role;
  permissions: Permission[];
  amr: string[];
  ip: string;
  userAgent: string;
}

function secret(): Uint8Array {
  const s = process.env.NEXTAUTH_SECRET;
  if (!s) throw new Error('NEXTAUTH_SECRET is not set');
  return new TextEncoder().encode(s);
}

export function clientIp(req: Request): string {
  return req.headers.get('x-forwarded-for')?.split(',')[0]?.trim() || req.headers.get('x-real-ip') || 'unknown';
}

export async function signSession(user: UserRecord, orgId: string, amr: string[]): Promise<string> {
  return new SignJWT({ name: user.name, uid: user.id, org: orgId, amr })
    .setProtectedHeader({ alg: 'HS256' })
    .setSubject(user.email)
    .setIssuedAt()
    .setExpirationTime(`${SESSION_HOURS}h`)
    .sign(secret());
}

/** Put a session cookie on a response. */
export async function attachSession<T extends NextResponse>(res: T, user: UserRecord, orgId: string, amr: string[]): Promise<T> {
  res.cookies.set(SESSION_COOKIE, await signSession(user, orgId, amr), {
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
const MFA_SETUP_PATHS = ['/api/account/mfa', '/api/auth/session', '/api/auth/logout'];

export async function authenticate(req: NextRequest): Promise<AuthContext | NextResponse> {
  if (!process.env.NEXTAUTH_SECRET) return deny(500, 'Server configuration error');
  const token = req.cookies.get(SESSION_COOKIE)?.value;
  if (!token) return deny(401, 'Unauthorized');
  let payload: Record<string, unknown>;
  try {
    payload = (await jwtVerify(token, secret())).payload as Record<string, unknown>;
  } catch {
    return deny(401, 'Session expired');
  }
  const user = typeof payload.uid === 'string' ? getUser(payload.uid) : null;
  if (!user || !user.isActive) return deny(401, 'Account is no longer active');
  const orgId = typeof payload.org === 'string' ? payload.org : user.organizationId;
  const org = getOrg(orgId);
  const role = org ? effectiveRole(user.id, orgId) : null;
  if (!org || !role) return deny(403, 'You no longer have access to this organization');
  const amr = Array.isArray(payload.amr) ? (payload.amr as string[]) : ['pwd'];
  // An organization requires two-factor sign-in and this account has none yet:
  // the session can only set it up (also for sessions from before the rule).
  if (!user.mfaEnabled && mfaRequired(user, amr.includes('sso')) && !MFA_SETUP_PATHS.some((p) => req.nextUrl.pathname.startsWith(p))) {
    return NextResponse.json({ error: 'Your organization requires two-factor sign-in. Set it up under My account first.', code: 'mfa_setup_required' }, { status: 403 });
  }
  return {
    user,
    orgId,
    role,
    permissions: ROLE_PERMISSIONS[role] ?? [],
    amr,
    ip: clientIp(req),
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
  if (!can(ctx, 'admin.access')) return deny(403, 'Your role cannot use the admin console');
  const missing = perms.filter((p) => !can(ctx, p));
  if (missing.length) return deny(403, `Forbidden: requires ${missing.join(', ')}`);
  return ctx;
}

/** Permission check against another organization the user can reach. */
export function canIn(ctx: AuthContext, orgId: string, p: Permission): boolean {
  if (orgId === ctx.orgId) return can(ctx, p);
  const role = effectiveRole(ctx.user.id, orgId);
  return !!role && (ROLE_PERMISSIONS[role] ?? []).includes(p);
}

export function audit(ctx: AuthContext | null, e: AuditInput): void {
  appendAudit({
    orgId: ctx?.orgId,
    userId: ctx?.user.id,
    userEmail: ctx?.user.email,
    userName: ctx?.user.name,
    ipAddress: ctx?.ip,
    userAgent: ctx?.userAgent,
    ...e,
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
