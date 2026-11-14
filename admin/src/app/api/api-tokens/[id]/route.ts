import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, can, isContext, requirePermission } from '@/lib/session';
import { getApiToken, revokeApiToken } from '@/lib/db/repos/api-tokens';

/** DELETE /api/api-tokens/:id — revoke your token, or (users.write) any member's in this organization. */
export async function DELETE(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const t = getApiToken((await params).id);
  if (!t || t.orgId !== ctx.orgId || (t.userId !== ctx.user.id && !can(ctx, 'users.write'))) return badRequest('Token not found', 404);
  if (!revokeApiToken(t.id)) return badRequest('The token is already revoked', 409);
  audit(ctx, {
    eventType: 'api_token.revoke',
    severity: 'warning',
    description: `Revoked API token "${t.name}"${t.userId !== ctx.user.id ? ' of another member' : ''}`,
    resourceType: 'api_token',
    resourceId: t.id,
  });
  return NextResponse.json({ success: true });
}
