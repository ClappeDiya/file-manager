import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, requireStepUp } from '@/lib/session';
import { revokeScimToken } from '@/lib/db/repos/scim';

/** DELETE /api/scim-tokens/:id — revoke (the identity provider can no longer provision). */
export async function DELETE(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requireStepUp(request, 'users.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!revokeScimToken(ctx.orgId, id)) return badRequest('Token not found', 404);
  audit(ctx, { eventType: 'scim.token_revoke', severity: 'warning', description: `Revoked SCIM token ${id}`, resourceType: 'scim_token', resourceId: id });
  return NextResponse.json({ success: true });
}
