import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { revokeLink } from '@/lib/db/repos/machines';

/** DELETE /api/fleet/links/:id — revoke (machines already registered keep working). */
export async function DELETE(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'fleet.enroll');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!revokeLink(ctx.orgId, id)) return badRequest('No active link with this id', 404);
  audit(ctx, { eventType: 'fleet.link_revoke', severity: 'warning', description: `Revoked registration link ${id}`, resourceType: 'registration_link', resourceId: id });
  return NextResponse.json({ success: true });
}
