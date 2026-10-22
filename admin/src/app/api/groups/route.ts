import { NextRequest, NextResponse } from 'next/server';
import { ROLE_HIERARCHY, type Role } from '@/lib/types/auth';
import { audit, badRequest, can, isContext, readJson, requirePermission } from '@/lib/session';
import { createGroup, groupGrants, groupMembers, isGroupOwner, listGroups } from '@/lib/db/repos/groups';
import { OrgError } from '@/lib/db/repos/orgs';
import { canManageRole } from '@/lib/utils/permissions';

/** GET /api/groups — groups of the current organization with members and grants. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const groups = listGroups(ctx.orgId).map((g) => ({ ...g, members: groupMembers(g.id), grants: groupGrants(g.id), canManage: can(ctx, 'groups.write') || isGroupOwner(g.id, ctx.user.id) }));
  return NextResponse.json({ groups });
}

/** POST /api/groups {name, description?, orgRole?, delegateToChildren?} — the creator becomes an owner. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'groups.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ name?: string; description?: string; orgRole?: Role | null; delegateToChildren?: boolean }>(request);
  if (!b?.name) return badRequest('Give the group a name');
  if (b.orgRole && (!(b.orgRole in ROLE_HIERARCHY) || (ctx.role !== 'org_owner' && !canManageRole(ctx.role, b.orgRole)))) return badRequest('You cannot grant that role', 403);
  try {
    const g = createGroup(ctx.orgId, { name: b.name, description: b.description, orgRole: b.orgRole ?? null, delegateToChildren: !!b.delegateToChildren });
    const { setGroupMember } = await import('@/lib/db/repos/groups');
    setGroupMember(g.id, ctx.user.id, 'owner');
    audit(ctx, { eventType: 'group.create', description: `Created group "${g.name}"`, resourceType: 'group', resourceId: g.id, metadata: { orgRole: g.orgRole, delegate: g.delegateToChildren } });
    return NextResponse.json({ group: g }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
}
