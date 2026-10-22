import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, badRequest, isContext, readJson } from '@/lib/session';
import { beginEnrollment, confirmEnrollment, disableMfa } from '@/lib/db/repos/mfa';
import { getUser, updateUser } from '@/lib/db/repos/users';
import { open } from '@/lib/crypto';
import { otpauthUrl, verifyTotp } from '@/lib/totp';
import { mfaRequired } from '@/lib/db/repos/security';

/** GET /api/account/mfa — whether two-factor sign-in is on. */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ enabled: ctx.user.mfaEnabled, pending: !!ctx.user.totpPendingCiphertext, recoveryCodesLeft: ctx.user.recoveryCodes.length });
}

/**
 * POST /api/account/mfa {action: begin | confirm | disable, code?}
 * Disabling needs a current code, so a stolen session cannot switch it off.
 */
export async function POST(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ action?: string; code?: string }>(request);
  const user = ctx.user;
  switch (b?.action) {
    case 'begin': {
      if (user.mfaEnabled) return badRequest('Two-factor sign-in is already on', 409);
      const secret = beginEnrollment(user);
      return NextResponse.json({ secret, otpauthUrl: otpauthUrl(secret, user.email) });
    }
    case 'confirm': {
      const r = confirmEnrollment(getUser(user.id)!, String(b.code ?? ''));
      if (!r.ok) return badRequest(r.error);
      audit(ctx, { eventType: 'auth.mfa_enable', description: 'Turned on two-factor sign-in', resourceType: 'user', resourceId: user.id });
      return NextResponse.json({ enabled: true, recoveryCodes: r.recoveryCodes });
    }
    case 'disable': {
      if (!user.mfaEnabled || !user.totpSecretCiphertext) return badRequest('Two-factor sign-in is not on', 409);
      if (mfaRequired(user, false)) return badRequest('Your organization requires two-factor sign-in', 403);
      const step = verifyTotp(open(user.totpSecretCiphertext), String(b.code ?? ''), user.lastTotpStep);
      if (step === null) return badRequest('The code is not valid', 401);
      updateUser(user.id, { lastTotpStep: step });
      disableMfa(user);
      audit(ctx, { eventType: 'auth.mfa_disable', severity: 'warning', description: 'Turned off two-factor sign-in', resourceType: 'user', resourceId: user.id });
      return NextResponse.json({ enabled: false });
    }
    default:
      return badRequest('action must be begin, confirm or disable');
  }
}
