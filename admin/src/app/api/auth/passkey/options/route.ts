import { NextRequest, NextResponse } from 'next/server';
import { createCeremony } from '@/lib/db/repos/passkeys';
import { relyingParty } from '@/lib/rp';

/** POST /api/auth/passkey/options — a challenge for signing in with a passkey (discoverable credentials). */
export async function POST(request: NextRequest) {
  const c = createCeremony('login', null);
  return NextResponse.json({ challengeId: c.id, publicKey: { challenge: c.challenge, rpId: relyingParty(request).id, userVerification: 'preferred', timeout: 300_000 } });
}
