import { NextRequest, NextResponse } from 'next/server';
import { attachSession, audit, authenticate, badRequest, isContext, readJson } from '@/lib/session';
import { effectiveRole, getOrg } from '@/lib/db/repos/orgs';

/** POST /api/orgs/switch {orgId} — re-issue the session scoped to another organization. */
export async function POST(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ orgId?: string }>(request);
  const org = b?.orgId ? getOrg(b.orgId) : null;
  const role = org ? effectiveRole(ctx.user.id, org.id) : null;
  if (!org || !role) return badRequest('You do not have access to that organization', 403);
  audit({ ...ctx, orgId: org.id }, { eventType: 'org.switch', description: `Switched to "${org.name}"`, resourceType: 'organization', resourceId: org.id });
  return attachSession(NextResponse.json({ orgId: org.id, role }), ctx.user, org.id, ctx.amr);
}
