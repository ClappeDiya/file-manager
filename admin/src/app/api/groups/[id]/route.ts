import { NextRequest, NextResponse } from 'next/server';
import { ROLE_HIERARCHY, type Role } from '@/lib/types/auth';
import { audit, badRequest, canIn, isContext, readJson, requirePermission } from '@/lib/session';
import { deleteGroup, getGroup, updateGroup } from '@/lib/db/repos/groups';
import { canManageRole } from '@/lib/utils/permissions';

type P = { params: Promise<{ id: string }> };

/** PATCH /api/groups/:id — rename, describe, change the granted role or delegation (admins only). */
export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const g = getGroup(id);
  if (!g || !canIn(ctx, g.orgId, 'groups.write')) return badRequest('Group not found or not manageable', 404);
  const b = await readJson<{ name?: string; description?: string; orgRole?: Role | null; delegateToChildren?: boolean }>(request);
  if (!b) return badRequest('Invalid request');
  if (b.orgRole && (!(b.orgRole in ROLE_HIERARCHY) || (ctx.role !== 'org_owner' && !canManageRole(ctx.role, b.orgRole)))) return badRequest('You cannot grant that role', 403);
  const upd = updateGroup(id, b)!;
  audit(ctx, { eventType: 'group.update', description: `Updated group "${upd.name}"`, resourceType: 'group', resourceId: id, metadata: { fields: Object.keys(b) } });
  return NextResponse.json({ group: upd });
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const g = getGroup(id);
  if (!g || !canIn(ctx, g.orgId, 'groups.write')) return badRequest('Group not found or not manageable', 404);
  deleteGroup(id);
  audit(ctx, { eventType: 'group.delete', severity: 'warning', description: `Deleted group "${g.name}"`, resourceType: 'group', resourceId: id });
  return NextResponse.json({ success: true });
}
