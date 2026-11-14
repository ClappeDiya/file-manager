import { NextRequest, NextResponse } from 'next/server';
import { ROLE_HIERARCHY, type Role } from '@/lib/types/auth';
import { authenticate, isContext, can, type AuthContext } from '@/lib/session';

/** Coarse levels used by the older routes; mapped onto the real role hierarchy. */
export type UserRole = 'admin' | 'manager' | 'viewer';

const MIN_RANK: Record<UserRole, number> = {
  admin: ROLE_HIERARCHY.security_admin,
  manager: ROLE_HIERARCHY.operator,
  viewer: ROLE_HIERARCHY.read_only_auditor,
};

interface SessionUser {
  id: string;
  email: string;
  name: string;
  role: Role;
}

export async function requireRole(request: NextRequest, minimumRole: UserRole): Promise<{ user: SessionUser; ctx: AuthContext } | NextResponse> {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  if (!can(ctx, 'admin.access') || (ROLE_HIERARCHY[ctx.role] ?? 0) < MIN_RANK[minimumRole]) {
    return NextResponse.json({ error: 'Forbidden: insufficient role' }, { status: 403 });
  }
  return { user: { id: ctx.user.id, email: ctx.user.email, name: ctx.user.name, role: ctx.role }, ctx };
}

export function isAuthorized(result: { user: SessionUser; ctx: AuthContext } | NextResponse): result is { user: SessionUser; ctx: AuthContext } {
  return !(result instanceof NextResponse);
}
