import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { completeChallenge } from '@/lib/db/repos/mfa';
import { appendAudit } from '@/lib/db/repos/audit';

/** POST /api/auth/mfa/verify {challengeId, code} — second step of sign-in. */
export async function POST(request: NextRequest) {
  const b = await readJson<{ challengeId?: string; code?: string; redirect?: string }>(request);
  if (!b?.challengeId || !b.code) return NextResponse.json({ error: 'Enter the code from your authenticator app' }, { status: 400 });
  const r = completeChallenge(b.challengeId, b.code);
  const ip = clientIp(request);
  if (!r.ok) {
    appendAudit({ eventType: 'auth.login_failed', severity: 'warning', ipAddress: ip, description: `Second factor rejected: ${r.error}` });
    return NextResponse.json({ error: r.error }, { status: r.status });
  }
  if (r.usedRecoveryCode) appendAudit({ eventType: 'auth.recovery_code_used', severity: 'warning', userId: r.user.id, userEmail: r.user.email, ipAddress: ip, description: 'Signed in with a recovery code' });
  const redirect = typeof b.redirect === 'string' && b.redirect.startsWith('/') && !b.redirect.startsWith('//') ? b.redirect : '/';
  return finishSignIn(r.user, { ip, userAgent: request.headers.get('user-agent') ?? '', method: r.challenge.method, redirect }, true);
}
