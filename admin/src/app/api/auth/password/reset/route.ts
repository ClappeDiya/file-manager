import { NextRequest, NextResponse } from 'next/server';
import { hash } from 'bcryptjs';
import { clientIp, readJson } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { getUser, updateUser } from '@/lib/db/repos/users';
import { consumePasswordToken, peekPasswordToken } from '@/lib/db/repos/security';
import { appendAudit } from '@/lib/db/repos/audit';
import { checkPassword } from '@/lib/password-policy';
import { enrollLimiter } from '@/lib/rate-limit';
import { revokeUserSessions } from '@/lib/db/repos/sessions';

/** GET /api/auth/password/reset?token= — whether the link is valid, and for whom. */
export async function GET(request: NextRequest) {
  const t = peekPasswordToken(new URL(request.url).searchParams.get('token') ?? '');
  const u = t && getUser(t.userId);
  if (!t || !u) return NextResponse.json({ error: 'This link is not valid or has expired' }, { status: 404 });
  return NextResponse.json({ email: u.email, purpose: t.purpose });
}

/** POST /api/auth/password/reset {token, password} — set the new password and sign in (MFA still applies). */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  if (enrollLimiter.wait(`pwdreset:${ip}`)) return NextResponse.json({ error: 'Too many attempts. Try again later.' }, { status: 429 });
  const b = await readJson<{ token?: string; password?: string }>(request);
  const peek = b?.token ? peekPasswordToken(b.token) : null;
  const user = peek && getUser(peek.userId);
  if (!peek || !user || !user.isActive) {
    enrollLimiter.fail(`pwdreset:${ip}`);
    return NextResponse.json({ error: 'This link is not valid or has expired' }, { status: 404 });
  }
  const check = checkPassword(b?.password ?? '', { email: user.email, name: user.name });
  if (!check.ok) return NextResponse.json({ error: check.problems[0], problems: check.problems }, { status: 400 });
  // Use the token only once the new password is acceptable.
  if (!consumePasswordToken(b!.token!)) return NextResponse.json({ error: 'This link is not valid or has expired' }, { status: 404 });
  updateUser(user.id, { passwordHash: await hash(b!.password!, 12), passwordChangedAt: new Date().toISOString() });
  revokeUserSessions(user.id, peek.purpose === 'expired' ? 'password changed' : 'password reset');
  appendAudit({ eventType: peek.purpose === 'expired' ? 'auth.password_change' : 'auth.password_reset', orgId: user.organizationId, userId: user.id, userEmail: user.email, ipAddress: ip, description: peek.purpose === 'expired' ? 'Changed an expired password' : 'Reset the password with an e-mailed link' });
  return finishSignIn(getUser(user.id)!, { ip, userAgent: request.headers.get('user-agent') ?? '', method: 'pwd' });
}
