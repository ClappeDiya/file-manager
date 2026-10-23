import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { ldapIdentity } from '@/lib/ldap';
import { providerOrThrow, resolveUser, SsoTrace } from '@/lib/sso';
import { recordSsoAttempt } from '@/lib/db/repos/sso';
import { OrgError } from '@/lib/db/repos/orgs';
import { appendAudit } from '@/lib/db/repos/audit';
import { loginLimiter } from '@/lib/rate-limit';
import { safeRedirect } from '@/lib/route-helpers';
import { logger } from '@/lib/utils/logger';

/**
 * POST /api/auth/ldap {providerId, username, password, redirect?} — sign in
 * with a directory account; answers like /api/auth/login (a session, or
 * `mfaRequired` with a challenge id).
 */
export async function POST(request: NextRequest) {
  if (!process.env.NEXTAUTH_SECRET) return NextResponse.json({ error: 'Server configuration error' }, { status: 500 });
  const b = await readJson<{ providerId?: string; username?: string; password?: string; redirect?: string }>(request);
  const username = b?.username?.trim().toLowerCase() ?? '';
  const ip = clientIp(request);
  const keys = [`ip:${ip}`, `ldap:${b?.providerId ?? ''}:${username}`];
  const wait = Math.max(...keys.map((k) => loginLimiter.wait(k)));
  if (wait) return NextResponse.json({ error: 'Too many login attempts. Please try again later.' }, { status: 429, headers: { 'Retry-After': String(wait) } });
  const trace = new SsoTrace();
  let provider;
  try {
    provider = providerOrThrow(b?.providerId ?? '');
    if (provider.kind !== 'ldap' || !provider.enabled) throw new OrgError('Unknown sign-in method', 404);
  } catch (e) {
    return NextResponse.json({ error: (e as OrgError).message }, { status: (e as OrgError).status ?? 404 });
  }
  try {
    const identity = await ldapIdentity(provider, b?.username ?? '', b?.password ?? '', trace);
    const user = resolveUser(provider, identity, null, ip, trace);
    const res = await finishSignIn(user, { ip, userAgent: request.headers.get('user-agent') ?? '', method: `ldap:${provider.id}`, requireMfa: provider.requireMfa, redirect: safeRedirect(b?.redirect ?? '/') });
    const body = (await res.clone().json()) as { error?: string; mfaRequired?: boolean };
    trace.ok('sign_in', { mfaRequired: !!body.mfaRequired, refused: body.error ?? null });
    recordSsoAttempt(provider, { ok: !body.error, stage: body.error ? 'sign_in' : 'done', error: body.error ?? null, steps: trace.steps, email: trace.email, userId: user.id, ip });
    return res;
  } catch (e) {
    const msg = e instanceof OrgError ? e.message : 'Sign-in failed';
    if (!(e instanceof OrgError)) logger.error('auth.ldap_error', { providerId: provider.id, error: (e as Error)?.message });
    for (const k of keys) loginLimiter.fail(k);
    recordSsoAttempt(provider, { ok: false, stage: trace.stage, error: msg, steps: trace.steps, email: trace.email, userId: null, ip });
    appendAudit({ eventType: 'auth.login_failed', severity: 'warning', orgId: provider.orgId, userEmail: username, ipAddress: ip, description: `Directory sign-in failed (${provider.name}): ${msg}`, metadata: { providerId: provider.id } });
    return NextResponse.json({ error: msg }, { status: e instanceof OrgError ? e.status : 500 });
  }
}
