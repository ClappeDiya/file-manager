import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, canIn, isContext, readJson, requirePermission } from '@/lib/session';
import { getGroup, setGroupGrants, groupGrants, type Grant } from '@/lib/db/repos/groups';

/** PUT /api/groups/:id/grants {grants: [{resourceType: machine|tag|backup, resourceId}]} — what members may see. */
export async function PUT(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const g = getGroup(id);
  if (!g || !canIn(ctx, g.orgId, 'groups.write')) return badRequest('Only administrators can change what a group can access', 403);
  const b = await readJson<{ grants?: Grant[] }>(request);
  if (!Array.isArray(b?.grants)) return badRequest('grants must be a list');
  setGroupGrants(id, b.grants);
  audit(ctx, { eventType: 'group.grants', description: `Set ${b.grants.length} access grant(s) on "${g.name}"`, resourceType: 'group', resourceId: id });
  return NextResponse.json({ grants: groupGrants(id) });
}
