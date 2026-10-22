import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, badRequest, isContext, readJson } from '@/lib/session';
import { addPasskey, consumeCeremony, createCeremony, deletePasskey, listPasskeys, publicPasskey, renamePasskey } from '@/lib/db/repos/passkeys';
import { mfaRequired } from '@/lib/db/repos/security';
import { OrgError } from '@/lib/db/repos/orgs';
import { ALGS, b64url, verifyRegistration, WebAuthnError } from '@/lib/webauthn';
import { relyingParty } from '@/lib/rp';

/** GET /api/account/passkeys — your passkeys (no key material). */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ passkeys: listPasskeys(ctx.user.id).map(publicPasskey) });
}

/**
 * POST /api/account/passkeys
 * - {action: 'options'} → creation options for navigator.credentials.create
 * - {action: 'register', challengeId, name, credential: {response: {clientDataJSON, attestationObject}, transports?}}
 */
export async function POST(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ action?: string; challengeId?: string; name?: string; credential?: { response?: { clientDataJSON?: string; attestationObject?: string }; transports?: string[] } }>(request);
  const rp = relyingParty(request);
  if (b?.action === 'options') {
    const c = createCeremony('register', ctx.user.id);
    return NextResponse.json({
      challengeId: c.id,
      publicKey: {
        rp: { id: rp.id, name: rp.name },
        user: { id: b64url(Buffer.from(ctx.user.id)), name: ctx.user.email, displayName: ctx.user.name },
        challenge: c.challenge,
        pubKeyCredParams: [ALGS.ES256, ALGS.EdDSA, ALGS.RS256].map((alg) => ({ type: 'public-key', alg })),
        excludeCredentials: listPasskeys(ctx.user.id).map((p) => ({ type: 'public-key', id: p.credentialId, transports: p.transports })),
        authenticatorSelection: { residentKey: 'required', userVerification: 'preferred' },
        attestation: 'none',
        timeout: 300_000,
      },
    });
  }
  if (b?.action === 'register') {
    const challenge = b.challengeId ? consumeCeremony(b.challengeId, 'register', ctx.user.id) : null;
    if (!challenge) return badRequest('The registration expired; try again', 400);
    const r = b.credential?.response;
    if (!r?.clientDataJSON || !r.attestationObject) return badRequest('The passkey response is incomplete');
    try {
      const v = verifyRegistration({ clientDataJSON: r.clientDataJSON, attestationObject: r.attestationObject }, { challenge, origin: rp.origin, rpId: rp.id });
      const transports = (b.credential?.transports ?? []).filter((t) => typeof t === 'string');
      const p = addPasskey({ credentialId: v.credentialId, userId: ctx.user.id, name: b.name ?? 'Passkey', publicKey: v.publicKey, alg: v.alg, signCount: v.signCount, transports, aaguid: v.aaguid });
      audit(ctx, { eventType: 'auth.passkey_add', description: `Added passkey "${p.name}"`, resourceType: 'user', resourceId: ctx.user.id });
      return NextResponse.json({ passkey: publicPasskey(p) }, { status: 201 });
    } catch (e) {
      if (e instanceof WebAuthnError || e instanceof OrgError) return badRequest(e.message, e instanceof OrgError ? e.status : 400);
      throw e;
    }
  }
  return badRequest('action must be options or register');
}

/** PATCH /api/account/passkeys {id, name} — rename. */
export async function PATCH(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ id?: string; name?: string }>(request);
  if (!b?.id || !renamePasskey(ctx.user.id, b.id, b.name ?? '')) return badRequest('Passkey not found', 404);
  return NextResponse.json({ success: true });
}

/** DELETE /api/account/passkeys?id= */
export async function DELETE(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const id = new URL(request.url).searchParams.get('id') ?? '';
  // The last second factor cannot go while an organization requires one.
  if (!ctx.user.mfaEnabled && listPasskeys(ctx.user.id).length === 1 && mfaRequired(ctx.user, false)) return badRequest('Your organization requires two-factor sign-in; set up an authenticator app or another passkey first', 403);
  if (!deletePasskey(ctx.user.id, id)) return badRequest('Passkey not found', 404);
  audit(ctx, { eventType: 'auth.passkey_remove', severity: 'warning', description: 'Removed a passkey', resourceType: 'user', resourceId: ctx.user.id });
  return NextResponse.json({ success: true });
}
