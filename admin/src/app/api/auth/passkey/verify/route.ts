import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { consumeCeremony, getPasskey, touchPasskey } from '@/lib/db/repos/passkeys';
import { getUser } from '@/lib/db/repos/users';
import { appendAudit } from '@/lib/db/repos/audit';
import { verifyAssertion, WebAuthnError } from '@/lib/webauthn';
import { relyingParty } from '@/lib/rp';
import { loginLimiter } from '@/lib/rate-limit';

/**
 * POST /api/auth/passkey/verify {challengeId, credential: {id, response: {clientDataJSON, authenticatorData, signature}}}
 * A passkey with user verification (PIN/biometric) counts as two factors.
 */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  const wait = loginLimiter.wait(`ip:${ip}`);
  if (wait) return NextResponse.json({ error: 'Too many login attempts. Please try again later.' }, { status: 429, headers: { 'Retry-After': String(wait) } });
  const b = await readJson<{ challengeId?: string; credential?: { id?: string; response?: { clientDataJSON?: string; authenticatorData?: string; signature?: string } } }>(request);
  const fail = (reason: string, userId = '', email = '') => {
    loginLimiter.fail(`ip:${ip}`);
    appendAudit({ eventType: 'auth.login_failed', severity: 'warning', userId, userEmail: email, ipAddress: ip, description: `Failed passkey sign-in (${reason})` });
    return NextResponse.json({ error: 'The passkey could not be verified' }, { status: 401 });
  };
  const challenge = b?.challengeId ? consumeCeremony(b.challengeId, 'login', null) : null;
  if (!challenge) return fail('expired or unknown challenge');
  const r = b?.credential?.response;
  const pk = b?.credential?.id ? getPasskey(b.credential.id) : null;
  if (!pk || !r?.clientDataJSON || !r.authenticatorData || !r.signature) return fail('unknown passkey');
  const user = getUser(pk.userId);
  if (!user) return fail('unknown passkey');
  const rp = relyingParty(request);
  let result;
  try {
    result = verifyAssertion({ clientDataJSON: r.clientDataJSON, authenticatorData: r.authenticatorData, signature: r.signature }, pk, { challenge, origin: rp.origin, rpId: rp.id });
  } catch (e) {
    if (e instanceof WebAuthnError) return fail(e.message, user.id, user.email);
    throw e;
  }
  if (!user.isActive) return fail('account deactivated', user.id, user.email);
  touchPasskey(pk.credentialId, result.signCount);
  return finishSignIn(user, { ip, userAgent: request.headers.get('user-agent') ?? '', method: 'passkey', redirect: '/' }, result.userVerified);
}
