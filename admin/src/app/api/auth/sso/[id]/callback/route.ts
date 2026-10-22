import { NextRequest, NextResponse } from 'next/server';
import { clientIp } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { oidcIdentity, providerOrThrow, resolveUser, samlIdentity, type IdpIdentity } from '@/lib/sso';
import { takeState, type AuthState } from '@/lib/db/repos/sso';
import { OrgError } from '@/lib/db/repos/orgs';
import { appendAudit } from '@/lib/db/repos/audit';
import { logger } from '@/lib/utils/logger';
import { publicOrigin, safeRedirect } from '@/lib/route-helpers';

type Params = { params: Promise<{ id: string }> };

/** Turn the JSON outcome of `finishSignIn` into a browser redirect. */
async function complete(request: NextRequest, providerId: string, identity: IdpIdentity, st: AuthState): Promise<NextResponse> {
  const origin = publicOrigin(request);
  const p = providerOrThrow(providerId);
  const ip = clientIp(request);
  const user = resolveUser(p, identity, st.linkUserId, ip);
  if (st.linkUserId) return NextResponse.redirect(`${origin}/account?linked=${encodeURIComponent(p.name)}`, 303);
  const redirect = safeRedirect(st.redirect);
  const res = await finishSignIn(user, { ip, userAgent: request.headers.get('user-agent') ?? '', method: `sso:${p.id}`, requireMfa: p.requireMfa, redirect });
  const body = (await res.clone().json()) as { mfaRequired?: boolean; challengeId?: string; error?: string };
  if (body.mfaRequired) return NextResponse.redirect(`${origin}/login?mfa=${encodeURIComponent(body.challengeId ?? '')}&redirect=${encodeURIComponent(redirect)}`, 303);
  if (body.error) return NextResponse.redirect(`${origin}/login?error=${encodeURIComponent(body.error)}`, 303);
  const out = NextResponse.redirect(`${origin}${redirect}`, 303);
  for (const c of res.cookies.getAll()) out.cookies.set(c);
  return out;
}

function failure(request: NextRequest, providerId: string, e: unknown): NextResponse {
  const msg = e instanceof OrgError ? e.message : 'Sign-in failed';
  if (!(e instanceof OrgError)) logger.error('auth.sso_error', { providerId, error: (e as Error)?.message });
  appendAudit({ eventType: 'auth.login_failed', severity: 'warning', ipAddress: clientIp(request), description: `SSO sign-in failed: ${msg}`, metadata: { providerId } });
  return NextResponse.redirect(`${publicOrigin(request)}/login?error=${encodeURIComponent(msg)}`, 303);
}

/** OIDC redirect: ?code=…&state=… (or ?error=…). */
export async function GET(request: NextRequest, { params }: Params) {
  const { id } = await params;
  const q = request.nextUrl.searchParams;
  try {
    if (q.get('error')) throw new OrgError(`The identity provider refused sign-in: ${q.get('error_description') ?? q.get('error')}`, 401);
    const st = takeState(q.get('state') ?? '');
    if (!st || st.providerId !== id) throw new OrgError('This sign-in link expired. Start again.', 401);
    const p = providerOrThrow(id);
    if (p.kind !== 'oidc') throw new OrgError('Wrong callback for this provider', 400);
    const code = q.get('code');
    if (!code) throw new OrgError('The identity provider sent no authorization code', 401);
    return await complete(request, id, await oidcIdentity(p, publicOrigin(request), code, st), st);
  } catch (e) {
    return failure(request, id, e);
  }
}

/** SAML assertion consumer service (HTTP-POST binding). */
export async function POST(request: NextRequest, { params }: Params) {
  const { id } = await params;
  try {
    const p = providerOrThrow(id);
    if (p.kind !== 'saml') throw new OrgError('Wrong callback for this provider', 400);
    const form = await request.formData();
    const body: Record<string, string> = {};
    for (const [k, v] of form.entries()) if (typeof v === 'string') body[k] = v;
    if (!body.SAMLResponse) throw new OrgError('No SAML response was posted', 400);
    const { identity, relayState } = await samlIdentity(p, publicOrigin(request), body);
    const st = takeState(relayState);
    if (!st || st.providerId !== id) throw new OrgError('This sign-in link expired. Start again.', 401);
    return await complete(request, id, identity, st);
  } catch (e) {
    return failure(request, id, e);
  }
}
