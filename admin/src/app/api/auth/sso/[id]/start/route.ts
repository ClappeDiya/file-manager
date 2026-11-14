import { NextRequest, NextResponse } from 'next/server';
import { authenticate, isContext } from '@/lib/session';
import { providerOrThrow, startUrl } from '@/lib/sso';
import { OrgError } from '@/lib/db/repos/orgs';
import { publicOrigin, safeRedirect } from '@/lib/route-helpers';

/**
 * GET /api/auth/sso/:id/start?redirect=/path — send the browser to the IdP.
 * `?link=1` links the provider to the signed-in account instead of signing in.
 */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const origin = publicOrigin(request);
  const redirect = safeRedirect(request.nextUrl.searchParams.get('redirect'));
  const fail = (msg: string) => NextResponse.redirect(`${origin}/login?error=${encodeURIComponent(msg)}`, 303);
  try {
    const p = providerOrThrow(id);
    let linkUserId: string | null = null;
    if (request.nextUrl.searchParams.get('link') === '1') {
      const ctx = await authenticate(request);
      if (!isContext(ctx)) return fail('Sign in before linking a sign-in method');
      linkUserId = ctx.user.id;
    }
    return NextResponse.redirect(await startUrl(p, origin, linkUserId ? '/account' : redirect, linkUserId), 303);
  } catch (e) {
    if (e instanceof OrgError) return fail(e.message);
    return fail('Could not reach the identity provider');
  }
}
