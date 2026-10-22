import { NextRequest, NextResponse } from "next/server";
import { compare } from "bcryptjs";
import {
  audit,
  authenticate,
  badRequest,
  isContext,
  readJson,
  reauthMethods,
  type ReauthMethod,
} from "@/lib/session";
import { markVerified } from "@/lib/db/repos/sessions";
import { verifySecondFactor } from "@/lib/db/repos/mfa";
import {
  consumeCeremony,
  createCeremony,
  getPasskey,
  listPasskeys,
  touchPasskey,
} from "@/lib/db/repos/passkeys";
import { verifyAssertion, WebAuthnError } from "@/lib/webauthn";
import { relyingParty } from "@/lib/rp";
import { loginLimiter } from "@/lib/rate-limit";

interface Body {
  method?: ReauthMethod | "passkey_options";
  password?: string;
  code?: string;
  challengeId?: string;
  credential?: {
    id?: string;
    response?: {
      clientDataJSON?: string;
      authenticatorData?: string;
      signature?: string;
    };
  };
}

/**
 * POST /api/auth/reauth — confirm it is still you (step-up) before a sensitive change.
 *   {method: 'password', password} | {method: 'totp', code} (or a recovery code)
 *   {method: 'passkey_options'} → a challenge, then {method: 'passkey', challengeId, credential}
 * Only the account's strongest methods are accepted (see `reauthMethods`).
 */
export async function POST(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const key = `reauth:${ctx.user.id}`;
  const wait = loginLimiter.wait(key);
  if (wait)
    return NextResponse.json(
      { error: "Too many attempts. Please try again later." },
      { status: 429, headers: { "Retry-After": String(wait) } },
    );
  const b = (await readJson<Body>(request)) ?? {};
  const allowed = reauthMethods(ctx.user);
  const rp = relyingParty(request);

  if (b.method === "passkey_options") {
    if (!allowed.includes("passkey"))
      return badRequest("This account has no passkey");
    const c = createCeremony("reauth", ctx.user.id);
    return NextResponse.json({
      challengeId: c.id,
      publicKey: {
        challenge: c.challenge,
        rpId: rp.id,
        userVerification: "required",
        timeout: 300_000,
        allowCredentials: listPasskeys(ctx.user.id).map((p) => ({
          type: "public-key",
          id: p.credentialId,
          transports: p.transports,
        })),
      },
    });
  }
  if (!b.method || !allowed.includes(b.method))
    return badRequest(`Confirm with ${allowed.join(" or ")}`);
  if (b.method === "sso")
    return badRequest("Sign in again with single sign-on");

  const fail = (reason: string) => {
    loginLimiter.fail(key);
    audit(ctx, {
      eventType: "auth.reauth_failed",
      severity: "warning",
      description: `Re-authentication failed (${reason})`,
      metadata: { method: b.method },
    });
    return NextResponse.json(
      { error: "That did not work. Try again." },
      { status: 401 },
    );
  };

  if (b.method === "password") {
    if (
      !b.password ||
      !ctx.user.passwordHash ||
      !(await compare(b.password, ctx.user.passwordHash))
    )
      return fail("wrong password");
  } else if (b.method === "totp") {
    if (!b.code || !verifySecondFactor(ctx.user, b.code.replace(/\s/g, "")))
      return fail("wrong code");
  } else {
    const challenge = b.challengeId
      ? consumeCeremony(b.challengeId, "reauth", ctx.user.id)
      : null;
    const r = b.credential?.response;
    const pk = b.credential?.id ? getPasskey(b.credential.id) : null;
    if (!challenge) return fail("expired or unknown challenge");
    if (
      !pk ||
      pk.userId !== ctx.user.id ||
      !r?.clientDataJSON ||
      !r.authenticatorData ||
      !r.signature
    )
      return fail("unknown passkey");
    try {
      const v = verifyAssertion(
        {
          clientDataJSON: r.clientDataJSON,
          authenticatorData: r.authenticatorData,
          signature: r.signature,
        },
        pk,
        { challenge, origin: rp.origin, rpId: rp.id },
      );
      if (!v.userVerified) return fail("passkey without user verification");
      touchPasskey(pk.credentialId, v.signCount);
    } catch (e) {
      if (e instanceof WebAuthnError) return fail(e.message);
      throw e;
    }
  }
  markVerified(ctx.sid);
  audit(ctx, {
    eventType: "auth.reauth",
    description: `Confirmed identity (${b.method})`,
    resourceType: "session",
    resourceId: ctx.sid,
  });
  return NextResponse.json({ ok: true });
}
