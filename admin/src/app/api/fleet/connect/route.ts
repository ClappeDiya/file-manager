import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createConnectCode } from '@/lib/db/repos/machines';
import { OrgError } from '@/lib/db/repos/orgs';

/**
 * POST /api/fleet/connect {redirectUri, state, codeChallenge, name?, tags?}
 * — the signed-in user approves enrolling the desktop that opened this
 * sign-in; returns the loopback URL (with a one-time code) to go back to.
 */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.enroll');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ redirectUri?: string; state?: string; codeChallenge?: string; name?: string; tags?: string[] }>(request);
  if (!b?.redirectUri || !b.state || !b.codeChallenge) return badRequest('redirectUri, state and codeChallenge are required');
  try {
    const redirect = createConnectCode(ctx.orgId, ctx.user.email, { redirectUri: b.redirectUri, state: b.state, codeChallenge: b.codeChallenge, name: b.name, tags: b.tags });
    audit(ctx, { eventType: 'fleet.desktop_connect', description: `Approved enrolling desktop "${b.name ?? ''}" by browser sign-in`, resourceType: 'organization', resourceId: ctx.orgId });
    return NextResponse.json({ redirect });
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
}
