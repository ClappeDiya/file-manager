/** Final step of every sign-in path (password, SSO, MFA). */
import { NextResponse } from 'next/server';
import { attachSession, defaultOrgFor } from './session';
import { updateUser, type UserRecord } from './db/repos/users';
import { createChallenge } from './db/repos/mfa';
import { appendAudit } from './db/repos/audit';

export interface SignInMeta {
  ip: string;
  userAgent: string;
  /** `pwd` or `sso:<provider id>` */
  method: string;
  /** The identity provider insists on a second factor. */
  requireMfa?: boolean;
  redirect?: string;
}

/**
 * Either ask for the second factor (MFA enabled, or required by the provider)
 * or issue the session. A provider that requires MFA for a user without TOTP
 * is refused: enrollment must happen from an already strong session.
 */
export async function finishSignIn(user: UserRecord, meta: SignInMeta, mfaDone = false): Promise<NextResponse> {
  const orgId = defaultOrgFor(user);
  if (!orgId) return NextResponse.json({ error: 'Your account has no access to any organization' }, { status: 403 });
  const needsMfa = !mfaDone && (user.mfaEnabled || meta.requireMfa);
  if (needsMfa) {
    if (!user.mfaEnabled) {
      appendAudit({ eventType: 'auth.login_failed', severity: 'warning', orgId, userId: user.id, userEmail: user.email, ipAddress: meta.ip, description: 'Sign-in refused: the identity provider requires two-factor authentication, which is not set up', metadata: { method: meta.method } });
      return NextResponse.json({ error: 'This sign-in method requires two-factor authentication. Set it up after signing in with your password, or ask an administrator.' }, { status: 403 });
    }
    const ch = createChallenge(user.id, orgId, meta.method);
    return NextResponse.json({ mfaRequired: true, challengeId: ch.id, expiresAt: ch.expiresAt, redirect: meta.redirect ?? '/' });
  }
  updateUser(user.id, { lastLoginAt: new Date().toISOString() });
  const amr = [meta.method.startsWith('sso') ? 'sso' : 'pwd', ...(mfaDone ? ['mfa'] : [])];
  appendAudit({ eventType: 'auth.login', orgId, userId: user.id, userEmail: user.email, userName: user.name, ipAddress: meta.ip, userAgent: meta.userAgent, description: `Signed in (${amr.join(' + ')})`, metadata: { method: meta.method } });
  const res = NextResponse.json({ user: { id: user.id, email: user.email, name: user.name }, orgId, redirect: meta.redirect ?? '/' });
  return attachSession(res, user, orgId, amr);
}
