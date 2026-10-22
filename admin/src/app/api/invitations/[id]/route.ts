import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { revokeInvitation } from '@/lib/db/repos/invitations';

/** DELETE /api/invitations/:id — revoke a pending invitation. */
export async function DELETE(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'users.invite');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!revokeInvitation(ctx.orgId, id)) return badRequest('No pending invitation with this id', 404);
  audit(ctx, { eventType: 'admin.invite_revoke', description: `Revoked invitation ${id}`, resourceType: 'invitation', resourceId: id });
  return NextResponse.json({ success: true });
}
