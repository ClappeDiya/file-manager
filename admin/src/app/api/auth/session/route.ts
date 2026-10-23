import { NextRequest, NextResponse } from 'next/server';
import { authenticate, isContext } from '@/lib/session';
import { accessibleOrgs, getOrg, orgPath } from '@/lib/db/repos/orgs';
import { publicUser } from '@/lib/db/repos/users';

/** GET /api/auth/session — the signed-in user, current organization, role and permissions. */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const org = getOrg(ctx.orgId)!;
  return NextResponse.json({
    user: { ...publicUser(ctx.user), role: ctx.role },
    organization: { id: org.id, name: org.name, path: orgPath(org.id), status: org.status },
    role: ctx.role,
    permissions: ctx.permissions,
    amr: ctx.amr,
    // Read-only "view as": who is really signed in, and until when.
    viewAs: ctx.viewAs ? { actor: publicUser(ctx.viewAs.actor), until: ctx.viewAs.until } : null,
    organizations: accessibleOrgs(ctx.user.id).map(({ org: o, role }) => ({ id: o.id, name: o.name, path: orgPath(o.id), role, parentId: o.parentId })),
  });
}
