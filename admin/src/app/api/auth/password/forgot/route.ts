import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { findUserByEmail } from '@/lib/db/repos/users';
import { membershipsOf } from '@/lib/db/repos/orgs';
import { createPasswordToken } from '@/lib/db/repos/security';
import { appendAudit } from '@/lib/db/repos/audit';
import { sendPasswordResetEmail } from '@/lib/mailer';
import { loginLimiter } from '@/lib/rate-limit';
import { publicOrigin } from '@/lib/route-helpers';

/**
 * POST /api/auth/password/forgot {email} — e-mail a reset link (1 hour).
 * The answer is the same whether or not the account exists.
 */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  const b = await readJson<{ email?: string }>(request);
  const email = b?.email?.trim().toLowerCase() ?? '';
  const wait = Math.max(loginLimiter.wait(`reset-ip:${ip}`), loginLimiter.wait(`reset:${email}`));
  if (wait) return NextResponse.json({ error: 'Too many requests. Please try again later.' }, { status: 429, headers: { 'Retry-After': String(wait) } });
  // Every request counts, so the endpoint cannot be used to spam an inbox.
  loginLimiter.fail(`reset-ip:${ip}`);
  loginLimiter.fail(`reset:${email}`);
  const user = email ? findUserByEmail(email) : null;
  if (user && user.isActive && user.passwordHash) {
    const token = createPasswordToken(user.id, 'reset');
    const orgIds = membershipsOf(user.id).map((m) => m.orgId);
    const d = await sendPasswordResetEmail(orgIds, user.email, `${publicOrigin(request)}/reset-password?token=${encodeURIComponent(token)}`);
    appendAudit({ eventType: 'auth.password_reset_request', orgId: user.organizationId, userId: user.id, userEmail: user.email, ipAddress: ip, description: d.sent ? 'Password reset link sent' : 'Password reset requested; no account e-mail channel is set up', metadata: { emailed: d.sent } });
  }
  return NextResponse.json({ ok: true, message: 'If an account with a password exists for this address, a reset link is on its way.' });
}
