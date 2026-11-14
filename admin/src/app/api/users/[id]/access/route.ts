import { NextRequest, NextResponse } from 'next/server';
import { badRequest, can, isContext, requirePermission } from '@/lib/session';
import { explainAccess } from '@/lib/explain';
import { ROLE_PERMISSIONS, type Permission } from '@/lib/types/auth';

const ALL = new Set(Object.values(ROLE_PERMISSIONS).flat());

/**
 * GET /api/users/:id/access?permission= — why the user has (or lacks) access
 * in the current organization. Your own access needs no extra permission.
 */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (id !== ctx.user.id && !can(ctx, 'users.read')) return badRequest('Forbidden: requires users.read', 403);
  const p = new URL(request.url).searchParams.get('permission');
  if (p && !ALL.has(p as Permission)) return badRequest(`Unknown permission ${p}`);
  const e = explainAccess(id, ctx.orgId, (p as Permission) || undefined);
  // Only people connected to this organization (a member, or a grant from above).
  if (!e || (!e.sources.length && id !== ctx.user.id)) return badRequest('User not found in this organization', 404);
  return NextResponse.json(e);
}
